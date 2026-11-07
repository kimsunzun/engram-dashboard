//! The OS rules of desktop windows — window identity · position and z order · visibility · restoring position · topmost window under the cursor · window
//! diagnostic facts (the log's job) · whether the foreground window is this process's · whether that window is the foreground window · title-bar chrome shape (TRD S26-window-chrome §6-2 · §7-11).
//!
//! A window is received as `raw-window-handle` 0.6's [`HasWindowHandle`], and matching that handle's variant (Windows = `Win32` · otherwise =
//! [`WindowError::Unsupported`]) is done by this module — if the caller matched the variant, that too would be a runtime OS
//! split outside this crate (TRD S21-slot-docking 6.3rd edition §8 · B10 · 7th edition §5-1). The functions that change a window are raw Win32 calls, so they do not go through the window
//! toolkit's (tao's) flags — that it may disagree with the visibility the toolkit knows is the caller's contract (TRD
//! S21-slot-docking §9-3 I29).
//!
//! ★Threads★ — the functions that change a window ([`place_window`] · [`restore_placement`]) are called from the thread that created that window (main).
//! Called from another thread, Win32 sends a message to that window's thread and waits for the answer — if that thread is
//! waiting on the caller, it deadlocks. Reads ([`NativeWindow::of`] · [`top_level_at`] · [`describe_window`] ·
//! [`foreground_is_current_process`] · [`is_foreground`]) work from any thread.
//!
//! The snap layout overlay (a child window over the maximize button) is the submodule `snap` and sits behind the cargo feature `snap-overlay` — off by default (TRD
//! S26-window-chrome §8-4 · user decision Q1 2026-10-24).

// ADR-0266
// ADR-0295

use std::fmt;
use std::io;

use raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle};

// ADR-0304 — snap overlay feature
#[cfg(feature = "snap-overlay")]
pub mod snap;

/// A screen point in physical px. If the process is not per-monitor DPI aware (v2 — tao turns it on), the OS gives a value virtualized by scaling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PhysPoint {
    pub x: i32,
    pub y: i32,
}

/// A screen rectangle in physical px — top-left `(x, y)` and size. Whether it contains a point is half-open (`x ≤ 점 < x + width`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PhysRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Window identity — compare whether it is the same window (the answer of [`top_level_at`]) · used as the reference of [`ZOrder::After`]. The caller does not
/// know the inside (Windows = the HWND value).
///
/// ★The value remains even after the window is gone★ — the OS may give the same value again to a new window, so equality between long-held values does not
/// guarantee "the same window".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeWindow(isize);

impl NativeWindow {
    /// `None` = the handle could not be obtained (the window does not exist yet or any more) or it is not a window handle this crate handles — outside Windows it is
    /// always `None`.
    pub fn of(w: &impl HasWindowHandle) -> Option<NativeWindow> {
        raw_of(w).ok().map(NativeWindow)
    }
}

/// The log's job — the hexadecimal handle value that window-inspection tools (Spy++ and the like) use.
impl fmt::LowerHex for NativeWindow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::LowerHex::fmt(&self.0, f)
    }
}

/// The z position at which to place a window — the argument of [`place_window`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZOrder {
    /// The very top of the always-on-top band — makes that window always on top (Win32 `HWND_TOPMOST`).
    Topmost,
    /// The very top of the band the window is in (always-on-top · normal).
    Top,
    /// Directly below this window.
    After(NativeWindow),
    /// Does not touch the z order — but if `activate` of [`place_window`] is true, the OS raises the window that became active to the top.
    Keep,
}

/// Failure of a window function.
#[derive(Debug)]
pub enum WindowError {
    /// This OS has no such means — outside Windows, or the handle is not a Win32 window.
    Unsupported,
    /// The handle could not be obtained or that window is already gone.
    NoWindow,
    /// An OS call failed — the message holds that call's name and the OS error.
    Os(io::Error),
}

impl fmt::Display for WindowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => f.write_str("이 OS 에서는 창 함수가 없다"),
            Self::NoWindow => f.write_str("창이 없다"),
            Self::Os(e) => write!(f, "창 OS 호출 실패: {e}"),
        }
    }
}

impl std::error::Error for WindowError {}

/// Matches the handle's variant — the window functions handle only a Win32 window.
fn raw_of(w: &impl HasWindowHandle) -> Result<isize, WindowError> {
    let handle = w.window_handle().map_err(|e| match e {
        HandleError::NotSupported => WindowError::Unsupported,
        _ => WindowError::NoWindow,
    })?;
    match handle.as_raw() {
        RawWindowHandle::Win32(h) => Ok(h.hwnd.get()),
        _ => Err(WindowError::Unsupported),
    }
}

/// Applies position · size · z · visibility · activation with one `SetWindowPos` (TRD S21-slot-docking §9-3 I29 — places it in
/// one go without flicker).
///
/// - `rect: None` = leave position · size as is (show in place — used with `show`).
/// - `show` = if false, visibility is not touched (it does not hide).
/// - `activate` = if true, asks for that window to be the foreground — once active it comes to the top even if `z` is [`ZOrder::Keep`].
///   ★If the OS refuses, it is left as is and returns `Ok`★ — unlike tao's `force_window_active`, it does not synthesize Alt to steal the foreground:
///   it only raises without stealing focus (ADR-0299 decision 5 · TRD S21-slot-docking §9-3 I5 · I29). Whether it was refused is checked separately with [`is_foreground`] (is that window the foreground).
///
/// Outside Windows = [`WindowError::Unsupported`].
// ADR-0295
// ADR-0299
pub fn place_window(
    w: &impl HasWindowHandle,
    rect: Option<PhysRect>,
    z: ZOrder,
    show: bool,
    activate: bool,
) -> Result<(), WindowError> {
    #[cfg(windows)]
    {
        win::place_window(win::hwnd_of(w)?, rect, z, show, activate)
    }
    #[cfg(not(windows))]
    {
        let _ = (w, rect, z, show, activate);
        Err(WindowError::Unsupported)
    }
}

/// The topmost top-level window containing screen point `p` — walking down from the top of the z order, the first window that is visible · not minimized ·
/// not hidden (DWM cloaked — another virtual desktop and the like) · and not a see-through window. See-through window = a window that
/// clicks pass through (`WS_EX_LAYERED` + `WS_EX_TRANSPARENT` — any window) · among other apps' layered windows, one whose whole-window alpha is 0 or
/// that has a color key. Another app's layered window with per-pixel alpha counts as a covering window (decision = `hit_accepts`). Whether it is our
/// window is compared by the caller with [`NativeWindow::of`] — the drop command reads another app's window or the desktop as a new window
/// (TRD S21-slot-docking §3-5 ④ · PRD R22).
///
/// 「품는다」 (contains) means the OS outer rectangle (`GetWindowRect` — including the invisible resize border). The OS hit test also counts that
/// border as that window. But for a window with a window shape (`SetWindowRgn`), the point must be inside that shape. `None` = no window
/// contains it · outside Windows.
// ADR-0295
// ADR-0296
pub fn top_level_at(p: PhysPoint) -> Option<NativeWindow> {
    #[cfg(windows)]
    {
        win::top_level_at(p)
    }
    #[cfg(not(windows))]
    {
        let _ = p;
        None
    }
}

/// How see-through a layered window (Win32 `WS_EX_LAYERED`) is — as far as the OS lets us read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayeredKind {
    NotLayered,
    /// The whole-window alpha given with `SetLayeredWindowAttributes` — a window given without `LWA_ALPHA` is 255 (opaque).
    Alpha(u8),
    /// The color key (`LWA_COLORKEY`) of `SetLayeredWindowAttributes` — pixels of that color are not drawn and clicks pass through. Even if a whole-window
    /// alpha is also present, it is this.
    ColorKey,
    /// The attributes could not be read — for an `UpdateLayeredWindow` window with per-pixel alpha, the OS does not answer (MS
    /// `GetLayeredWindowAttributes` docs). A layered window with no attributes given yet also lands here (measured). Which pixels are empty is not known.
    PerPixel,
}

/// Diagnostic facts of one window — the answer of [`describe_window`] (the log's job).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowFacts {
    /// The process that created that window — 0 = it vanished in the meantime.
    pub pid: u32,
    /// Window class name — empty string if it could not be read.
    pub class: String,
    /// Extended style (Win32 `GWL_EXSTYLE` value) — 0 if it could not be read.
    pub ex_style: u32,
    /// If the extended style could not be read, it is `NotLayered`.
    pub layered: LayeredKind,
    /// Right-to-left layout (`WS_EX_LAYOUTRTL`) — window-shape coordinates are measured from the top right.
    pub rtl: bool,
    /// It has a window shape (Win32 `SetWindowRgn`).
    pub has_region: bool,
}

/// Diagnostic facts of that window — only queries that send no message to the window, so even an unresponsive other-app window does not block them. `None` =
/// that window does not exist · outside Windows.
pub fn describe_window(w: NativeWindow) -> Option<WindowFacts> {
    #[cfg(windows)]
    {
        win::describe(w)
    }
    #[cfg(not(windows))]
    {
        let _ = w;
        None
    }
}

/// Restores the normal position `normal` (★screen coordinates★) and maximization (`SetWindowPlacement` · TRD S21-slot-docking §5-1).
///
/// - Coordinate system — the normal position of `WINDOWPLACEMENT` is, for a top-level window without `WS_EX_TOOLWINDOW`, in workspace coordinates (the top left of that monitor's
///   work area is 0 — if the taskbar is at the top or left it is shifted by that much), and with it, in screen coordinates (MS
///   `WINDOWPLACEMENT` docs · TRD S21-slot-docking 6.3rd edition E12). This function converts it using that window's current extended style and the monitor nearest to `normal`
///   — so changing that window's `WS_EX_TOOLWINDOW` is finished before this function.
/// - `activate` = if false, the normal position is shown without activation (`SW_SHOWNOACTIVATE`). ★`maximized` is `SW_SHOWMAXIMIZED` regardless of
///   `activate`★ — Win32 has no show command that shows maximized without activation. So if the OS's foreground
///   rules allow, it becomes active (unconfirmed — TRD S21-slot-docking 6.3rd edition §12 M6 · §15-2 S-c).
/// - The OS fixes a normal position that goes entirely off screen back onto the screen (MS `SetWindowPlacement` docs).
///
/// Outside Windows = [`WindowError::Unsupported`].
// ADR-0295
pub fn restore_placement(
    w: &impl HasWindowHandle,
    normal: PhysRect,
    maximized: bool,
    activate: bool,
) -> Result<(), WindowError> {
    #[cfg(windows)]
    {
        win::restore_placement(win::hwnd_of(w)?, normal, maximized, activate)
    }
    #[cfg(not(windows))]
    {
        let _ = (w, normal, maximized, activate);
        Err(WindowError::Unsupported)
    }
}

/// Is the current foreground window this process's. `None` = this crate has no means to answer on this OS — the caller decides with its own
/// means (the window toolkit's focus getter and the like) (lib.rs 「들이는 규칙」 3 — the branch outside Windows is a placeholder).
///
/// Windows = is the PID of the thread that created the window of `GetForegroundWindow` the same as this process's PID. If there is no foreground window or it
/// vanished in the meantime, `Some(false)`.
#[cfg(windows)]
pub fn foreground_is_current_process() -> Option<bool> {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    let mut pid = 0u32;
    // SAFETY: one query with no arguments, and one query that writes a PID into a live local variable. If there is no foreground window or it
    //   vanished in the meantime, the PID stays 0.
    unsafe {
        let foreground = GetForegroundWindow();
        if !foreground.is_invalid() {
            GetWindowThreadProcessId(foreground, Some(&mut pid));
        }
    }
    Some(pid != 0 && pid == std::process::id())
}
#[cfg(not(windows))]
pub fn foreground_is_current_process() -> Option<bool> {
    None
}

/// Is that window the OS foreground window now — there is only one foreground window, so at most one window is true. `None` = this crate has no means to answer on this OS
/// — the caller decides with its own means (the window toolkit's focus getter and the like) (same as [`foreground_is_current_process`]).
///
/// Windows = is `GetForegroundWindow` that top-level window. If the handle could not be obtained or that window is already gone, `Some(false)`. It may
/// differ from the window toolkit's focus getter — for a window that hosts WebView2, that getter is false while the webview holds keyboard focus
/// (TRD S26-window-chrome §6-2 「앞 창 판정」).
// ADR-0302 — window state command (foreground window decision)
pub fn is_foreground(w: &impl HasWindowHandle) -> Option<bool> {
    #[cfg(windows)]
    {
        Some(win::is_foreground(w))
    }
    #[cfg(not(windows))]
    {
        let _ = w;
        None
    }
}

/// The chrome shape of a window's title bar — the answer of [`caption_style`].
///
/// - `drawn_buttons` = the app draws the minimize · maximize · close buttons (it strips the OS frame). False = the OS draws the buttons.
/// - `leading_inset` = the width (CSS px) the app leaves empty at the left of the title bar — the OS draws buttons in that spot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaptionStyle {
    pub drawn_buttons: bool,
    pub leading_inset: f64,
}

/// The title-bar chrome shape of this OS (TRD S26-window-chrome §7-11) — always `{ drawn_buttons: true, leading_inset: 0 }`. On macOS
/// the OS draws the traffic-light buttons, so it should be `{ false, <신호등 폭> }`, but that width was not measured and macOS is not built, so that
/// branch was not added — add it inside this function when building macOS.
// ADR-0303 — chrome-style title bar
pub fn caption_style() -> CaptionStyle {
    CaptionStyle {
        drawn_buttons: true,
        leading_inset: 0.0,
    }
}

/// A screen-coordinate rectangle to workspace coordinates — subtract the shift from the monitor's top left to the work area's top left. It has the same form as the opposite
/// computation Chromium uses when reading (hwnd_message_handler.cc `GetWindowPlacement` — it adds `rcWork − rcMonitor`).
fn screen_to_workspace(rect: PhysRect, monitor: PhysRect, work: PhysRect) -> PhysRect {
    PhysRect {
        x: rect.x - (work.x - monitor.x),
        y: rect.y - (work.y - monitor.y),
        ..rect
    }
}

fn contains(r: PhysRect, p: PhysPoint) -> bool {
    p.x >= r.x && p.x < r.x + r.width && p.y >= r.y && p.y < r.y + r.height
}

/// A point and that window's window shape (Win32 `SetWindowRgn`) — the material of [`hit_accepts`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegionHit {
    /// There is no window shape (or it could not be read) — the whole outer rectangle is that window.
    NoRegion,
    Inside,
    Outside,
}

/// Does it count a window containing the point in its outer rectangle (visible · not minimized · not hidden) as the answer of [`top_level_at`] — it follows the OS's
/// hit test as far as it can be read. `ours` = this process's window · `transparent` = `WS_EX_TRANSPARENT`.
///
/// - A layered + `WS_EX_TRANSPARENT` window is skipped — clicks pass through (MS 「Layered Windows」 · the OS `WindowFromPoint` also
///   skips it). A window with only `WS_EX_TRANSPARENT` on is a covering window — that flag alone is just drawing order and the window still receives the click (measured —
///   the test `transparent_only_window_still_takes_the_click`, compared against `WindowFromPoint`).
/// - Another app's layered window is skipped if whole-window alpha is 0 or it has a color key. The OS lets clicks pass through only at alpha 0 or color-key pixels
///   (MS 「Layered Windows」), but pixels cannot be read, so a color-key window is treated as see-through as a whole — the cost: the painted area of that window
///   is also treated as uncovered. This branch prevents a drop under an always-on-top recording-area frame (a screen-recording tool and the like) from being
///   read as a drop over another app's window and going to a new window instead of our window — it does not resolve if that frame is a per-pixel-alpha window, and which one it is was not measured.
/// - Any other app's layered window (alpha 1 or more · per-pixel alpha) is a covering window — many layered apps draw opaquely
///   (WPF `AllowsTransparency` · Electron `transparent` · Qt translucent windows). If all were skipped, the drop-end decision would read a drop over that app as a drop over
///   our window hidden under it, and no new window would be created (TRD S21-slot-docking §3-5 ③ · ④). Chromium `TopMostFinder` (Chrome 70 `window_finder_win.cc`) skips a window if either layered or
///   `WS_EX_TRANSPARENT` alone is on, but we do not follow it, to avoid paying this cost.
/// - Our window is skipped only if it is click-through — tao turns on layered only for cursor-ignoring windows (tao 0.35.3 `window_state.rs`
///   `IGNORE_CURSOR_EVENT`).
/// - If there is a window shape, the point must be inside it — outside the shape that window neither draws nor receives clicks (as in Chromium).
// ADR-0295
// ADR-0296
fn hit_accepts(ours: bool, layered: LayeredKind, transparent: bool, region: RegionHit) -> bool {
    let click_through = layered != LayeredKind::NotLayered && transparent;
    let see_through = click_through
        || (!ours && matches!(layered, LayeredKind::Alpha(0) | LayeredKind::ColorKey));
    !see_through && region != RegionHit::Outside
}

#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    use std::io;

    use raw_window_handle::HasWindowHandle;
    use windows::Win32::Foundation::{GetLastError, SetLastError, HWND, RECT, WIN32_ERROR};
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    use windows::Win32::Graphics::Gdi::{
        CreateRectRgn, DeleteObject, GetMonitorInfoW, GetWindowRgn, MonitorFromRect, PtInRegion,
        HRGN, MONITORINFO, MONITOR_DEFAULTTONEAREST, RGN_ERROR,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetForegroundWindow, GetLayeredWindowAttributes, GetTopWindow, GetWindow,
        GetWindowLongPtrW, GetWindowPlacement, GetWindowRect, GetWindowThreadProcessId, IsIconic,
        IsWindow, IsWindowVisible, SetForegroundWindow, SetWindowPlacement, SetWindowPos,
        GWL_EXSTYLE, GW_HWNDNEXT, HWND_TOP, HWND_TOPMOST, LAYERED_WINDOW_ATTRIBUTES_FLAGS,
        LWA_ALPHA, LWA_COLORKEY, SET_WINDOW_POS_FLAGS, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        SWP_NOZORDER, SWP_SHOWWINDOW, SW_SHOWMAXIMIZED, SW_SHOWNOACTIVATE, SW_SHOWNORMAL,
        WINDOWPLACEMENT, WINDOWPLACEMENT_FLAGS, WS_EX_LAYERED, WS_EX_LAYOUTRTL, WS_EX_TOOLWINDOW,
        WS_EX_TRANSPARENT,
    };

    use super::{
        contains, hit_accepts, raw_of, screen_to_workspace, LayeredKind, NativeWindow, PhysPoint,
        PhysRect, RegionHit, WindowError, WindowFacts, ZOrder,
    };

    /// Upper bound of the top-level window z walk — MS writes that walking with `GetWindow` can loop endlessly because the z order changes meanwhile
    /// (`GetWindow` docs). The top-level windows of one desktop, hidden ones included, do not exceed a few thousand.
    const MAX_Z_WALK: usize = 16_384;

    pub(super) fn hwnd_of(w: &impl HasWindowHandle) -> Result<HWND, WindowError> {
        let hwnd = HWND(raw_of(w)? as *mut c_void);
        // SAFETY: a query that only reads a value — a vanished handle gives false.
        if unsafe { IsWindow(hwnd) }.as_bool() {
            Ok(hwnd)
        } else {
            Err(WindowError::NoWindow)
        }
    }

    fn native(hwnd: HWND) -> NativeWindow {
        NativeWindow(hwnd.0 as isize)
    }

    pub(super) fn is_foreground(w: &impl HasWindowHandle) -> bool {
        let Ok(hwnd) = hwnd_of(w) else {
            return false;
        };
        // SAFETY: a query with no arguments.
        let foreground = unsafe { GetForegroundWindow() };
        foreground == hwnd
    }

    fn os(api: &str, e: windows::core::Error) -> WindowError {
        WindowError::Os(io::Error::other(format!("{api}: {e}")))
    }

    /// The `SetWindowPos` arguments of `place_window` — (z reference window, flags). `Keep`'s `SWP_NOZORDER` only makes the z reference window ignored; it does not
    /// stop activation from raising the window (MS `SetWindowPos` docs — without `SWP_NOACTIVATE`, activating takes it to the
    /// top of its band, and so does a following `SetForegroundWindow`).
    pub(super) fn place_plan(
        keep_rect: bool,
        z: ZOrder,
        show: bool,
        activate: bool,
    ) -> (HWND, SET_WINDOW_POS_FLAGS) {
        let mut flags = SET_WINDOW_POS_FLAGS(0);
        if keep_rect {
            flags |= SWP_NOMOVE | SWP_NOSIZE;
        }
        if show {
            flags |= SWP_SHOWWINDOW;
        }
        if !activate {
            flags |= SWP_NOACTIVATE;
        }
        let after = match z {
            ZOrder::Topmost => HWND_TOPMOST,
            ZOrder::Top => HWND_TOP,
            ZOrder::After(w) => HWND(w.0 as *mut c_void),
            ZOrder::Keep => {
                flags |= SWP_NOZORDER;
                HWND::default()
            }
        };
        (after, flags)
    }

    pub(super) fn place_window(
        hwnd: HWND,
        rect: Option<PhysRect>,
        z: ZOrder,
        show: bool,
        activate: bool,
    ) -> Result<(), WindowError> {
        let (after, flags) = place_plan(rect.is_none(), z, show, activate);
        let r = rect.unwrap_or_default();
        // SAFETY: only a live window handle (filtered by `hwnd_of`) and value arguments.
        unsafe { SetWindowPos(hwnd, after, r.x, r.y, r.width, r.height, flags) }
            .map_err(|e| os("SetWindowPos", e))?;
        if activate {
            // A refusal (false) is not a failure — see the `place_window` docs.
            // SAFETY: one live window handle.
            let _ = unsafe { SetForegroundWindow(hwnd) };
        }
        Ok(())
    }

    fn to_rect(r: RECT) -> PhysRect {
        PhysRect {
            x: r.left,
            y: r.top,
            width: r.right - r.left,
            height: r.bottom - r.top,
        }
    }

    fn to_win(r: PhysRect) -> RECT {
        RECT {
            left: r.x,
            top: r.y,
            right: r.x + r.width,
            bottom: r.y + r.height,
        }
    }

    fn ex_style(hwnd: HWND) -> Result<u32, WindowError> {
        // SAFETY: a live window handle. 0 can be a legitimate value, so clear the last error, then read, and look at it together (MS docs).
        unsafe {
            SetLastError(WIN32_ERROR(0));
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            if ex == 0 && GetLastError().0 != 0 {
                return Err(WindowError::Os(io::Error::last_os_error()));
            }
            Ok(ex as u32)
        }
    }

    pub(super) fn restore_placement(
        hwnd: HWND,
        normal: PhysRect,
        maximized: bool,
        activate: bool,
    ) -> Result<(), WindowError> {
        let tool = ex_style(hwnd)? & WS_EX_TOOLWINDOW.0 != 0;
        let stored = if tool {
            normal
        } else {
            let mut mi = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            // SAFETY: only pointers to a live local rectangle and struct. With `NEAREST` a monitor always exists.
            let ok = unsafe {
                let monitor = MonitorFromRect(&to_win(normal), MONITOR_DEFAULTTONEAREST);
                GetMonitorInfoW(monitor, &mut mi)
            };
            if !ok.as_bool() {
                return Err(WindowError::Os(io::Error::other("GetMonitorInfoW 실패")));
            }
            screen_to_workspace(normal, to_rect(mi.rcMonitor), to_rect(mi.rcWork))
        };
        let mut wp = WINDOWPLACEMENT {
            length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
            ..Default::default()
        };
        // SAFETY: a live window handle and a local struct with its length filled in.
        unsafe { GetWindowPlacement(hwnd, &mut wp) }.map_err(|e| os("GetWindowPlacement", e))?;
        wp.flags = WINDOWPLACEMENT_FLAGS(0);
        wp.rcNormalPosition = to_win(stored);
        let show = if maximized {
            SW_SHOWMAXIMIZED
        } else if activate {
            SW_SHOWNORMAL
        } else {
            SW_SHOWNOACTIVATE
        };
        wp.showCmd = show.0 as u32;
        // SAFETY: same as above.
        unsafe { SetWindowPlacement(hwnd, &wp) }.map_err(|e| os("SetWindowPlacement", e))?;
        Ok(())
    }

    fn cloaked(hwnd: HWND) -> bool {
        let mut v = 0u32;
        // SAFETY: a query that writes 4 bytes into a live local u32. If DWM is absent or it fails, the window counts as not hidden.
        unsafe {
            DwmGetWindowAttribute(
                hwnd,
                DWMWA_CLOAKED,
                &mut v as *mut u32 as *mut c_void,
                std::mem::size_of::<u32>() as u32,
            )
        }
        .is_ok()
            && v != 0
    }

    /// The process that created that window — 0 if the window vanished.
    fn pid_of(hwnd: HWND) -> u32 {
        let mut pid = 0u32;
        // SAFETY: a query that writes a PID into a live local variable — if the window vanished it stays 0.
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        pid
    }

    /// A copy of the window shape — `GetWindowRgn` copies it into a region the caller created. Deleted when dropped.
    struct Region(HRGN);

    impl Drop for Region {
        fn drop(&mut self) {
            // SAFETY: a region this value created and did not hand to a window (it only received a copy).
            let _ = unsafe { DeleteObject(self.0) };
        }
    }

    /// That window's window shape — `None` if there is none · it could not be read · or the region to receive it could not be created.
    fn window_region(hwnd: HWND) -> Option<Region> {
        // SAFETY: only value arguments — null on failure.
        let rgn = unsafe { CreateRectRgn(0, 0, 0, 0) };
        if rgn.is_invalid() {
            return None;
        }
        let rgn = Region(rgn);
        // SAFETY: a live region handle — `RGN_ERROR` if the window vanished or has no shape.
        (unsafe { GetWindowRgn(hwnd, rgn.0) } != RGN_ERROR).then_some(rgn)
    }

    fn region_hit(hwnd: HWND, outer: RECT, rtl: bool, p: PhysPoint) -> RegionHit {
        let Some(rgn) = window_region(hwnd) else {
            return RegionHit::NoRegion;
        };
        // Window-shape coordinates have 0 at the top left of the outer rectangle (MS `GetWindowRgn` docs). A `WS_EX_LAYOUTRTL` window has 0 at the top right
        //   and x grows to the left — the rightmost pixel is x = 0 (measured — the test `top_level_at_mirrors_the_region_of_rtl_windows`,
        //   compared against the OS hit test `WindowFromPoint`). Whether another app's window with a different DPI awareness reports its shape in that window's logical
        //   coordinates was not measured — if so, that window's shape decision is off by the scale factor.
        let x = if rtl {
            outer.right - 1 - p.x
        } else {
            p.x - outer.left
        };
        // SAFETY: a live region handle and value arguments.
        if unsafe { PtInRegion(rgn.0, x, p.y - outer.top) }.as_bool() {
            RegionHit::Inside
        } else {
            RegionHit::Outside
        }
    }

    fn layered_kind(hwnd: HWND, ex: u32) -> LayeredKind {
        if ex & WS_EX_LAYERED.0 == 0 {
            return LayeredKind::NotLayered;
        }
        let mut alpha = 0u8;
        let mut flags = LAYERED_WINDOW_ATTRIBUTES_FLAGS(0);
        // Whether it also answers for another process's window was not measured — if not received it is `PerPixel`, so it falls to a covering window (the new-window side for a drop).
        // SAFETY: a query that writes into a live local variable (the color key value is not taken). It fails if the window vanished.
        if unsafe { GetLayeredWindowAttributes(hwnd, None, Some(&mut alpha), Some(&mut flags)) }
            .is_err()
        {
            return LayeredKind::PerPixel;
        }
        if flags.0 & LWA_COLORKEY.0 != 0 {
            LayeredKind::ColorKey
        } else if flags.0 & LWA_ALPHA.0 != 0 {
            LayeredKind::Alpha(alpha)
        } else {
            LayeredKind::Alpha(u8::MAX)
        }
    }

    fn candidate(hwnd: HWND, p: PhysPoint) -> bool {
        let mut r = RECT::default();
        // SAFETY: for a window that vanished during the walk the query fails or returns false — that window is skipped.
        unsafe {
            if GetWindowRect(hwnd, &mut r).is_err() || !contains(to_rect(r), p) {
                return false;
            }
            if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
                return false;
            }
        }
        let Ok(ex) = ex_style(hwnd) else {
            return false;
        };
        let ours = pid_of(hwnd) == std::process::id();
        let transparent = ex & WS_EX_TRANSPARENT.0 != 0;
        let rtl = ex & WS_EX_LAYOUTRTL.0 != 0;
        hit_accepts(
            ours,
            layered_kind(hwnd, ex),
            transparent,
            region_hit(hwnd, r, rtl, p),
        ) && !cloaked(hwnd)
    }

    pub(super) fn describe(w: NativeWindow) -> Option<WindowFacts> {
        let hwnd = HWND(w.0 as *mut c_void);
        // SAFETY: a query that only reads a value — a vanished handle gives false.
        if !unsafe { IsWindow(hwnd) }.as_bool() {
            return None;
        }
        // A window class name does not exceed 256 characters (MS `WNDCLASSEXW` docs).
        let mut class = [0u16; 256];
        // SAFETY: a live local buffer — it wrote as many as the returned length (0 on failure).
        let len = unsafe { GetClassNameW(hwnd, &mut class) }.max(0) as usize;
        let ex = ex_style(hwnd).unwrap_or(0);
        Some(WindowFacts {
            pid: pid_of(hwnd),
            class: String::from_utf16_lossy(&class[..len.min(class.len())]),
            ex_style: ex,
            layered: layered_kind(hwnd, ex),
            rtl: ex & WS_EX_LAYOUTRTL.0 != 0,
            has_region: window_region(hwnd).is_some(),
        })
    }

    pub(super) fn top_level_at(p: PhysPoint) -> Option<NativeWindow> {
        // SAFETY: a query with no arguments (null = the desktop) — the top of the top-level window z order.
        let mut hwnd = unsafe { GetTopWindow(HWND::default()) }.ok()?;
        for _ in 0..MAX_Z_WALK {
            if candidate(hwnd, p) {
                return Some(native(hwnd));
            }
            // SAFETY: a value query — it fails if it is the bottom or the window vanished in the meantime.
            hwnd = unsafe { GetWindow(hwnd, GW_HWNDNEXT) }.ok()?;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── pure computation ──

    #[test]
    fn caption_style_has_the_app_draw_the_buttons_with_no_leading_inset() {
        assert_eq!(
            caption_style(),
            CaptionStyle {
                drawn_buttons: true,
                leading_inset: 0.0,
            }
        );
    }

    #[test]
    fn workspace_offset_follows_work_area_origin() {
        let monitor = PhysRect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let normal = PhysRect {
            x: 100,
            y: 200,
            width: 800,
            height: 600,
        };
        // (work area, expected) — below the taskbar · above · left · on the second monitor
        let cases = [
            (
                PhysRect {
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1040,
                },
                (100, 200),
            ),
            (
                PhysRect {
                    x: 0,
                    y: 40,
                    width: 1920,
                    height: 1040,
                },
                (100, 160),
            ),
            (
                PhysRect {
                    x: 60,
                    y: 0,
                    width: 1860,
                    height: 1080,
                },
                (40, 200),
            ),
        ];
        for (work, (x, y)) in cases {
            let got = screen_to_workspace(normal, monitor, work);
            assert_eq!(
                (got.x, got.y, got.width, got.height),
                (x, y, 800, 600),
                "work = {work:?}"
            );
        }
        let second = PhysRect {
            x: 1920,
            y: -200,
            width: 2560,
            height: 1440,
        };
        let second_work = PhysRect {
            x: 1920,
            y: -150,
            width: 2560,
            height: 1390,
        };
        let on_second = PhysRect {
            x: 2000,
            y: 0,
            width: 800,
            height: 600,
        };
        assert_eq!(
            screen_to_workspace(on_second, second, second_work),
            PhysRect {
                x: 2000,
                y: -50,
                width: 800,
                height: 600
            }
        );
    }

    #[test]
    fn contains_is_half_open() {
        let r = PhysRect {
            x: 10,
            y: 20,
            width: 5,
            height: 4,
        };
        assert!(contains(r, PhysPoint { x: 10, y: 20 }));
        assert!(contains(r, PhysPoint { x: 14, y: 23 }));
        assert!(!contains(r, PhysPoint { x: 15, y: 20 }));
        assert!(!contains(r, PhysPoint { x: 10, y: 24 }));
        assert!(!contains(r, PhysPoint { x: 9, y: 20 }));
    }

    #[test]
    fn hit_accepts_skips_see_through_foreign_windows_and_points_outside_the_region() {
        use LayeredKind::{Alpha, ColorKey, NotLayered, PerPixel};
        use RegionHit::{Inside, NoRegion, Outside};
        // (ours, layered, transparent, region, expected)
        let cases = [
            // Another app's window
            (false, NotLayered, false, NoRegion, true),
            (false, NotLayered, true, NoRegion, true),
            (false, Alpha(255), false, NoRegion, true),
            (false, Alpha(1), false, NoRegion, true),
            (false, Alpha(0), false, NoRegion, false),
            (false, ColorKey, false, NoRegion, false),
            (false, PerPixel, false, NoRegion, true),
            (false, Alpha(255), true, NoRegion, false),
            (false, PerPixel, true, NoRegion, false),
            // Our window — only click-through is skipped
            (true, NotLayered, false, NoRegion, true),
            (true, NotLayered, true, NoRegion, true),
            (true, Alpha(0), false, NoRegion, true),
            (true, ColorKey, false, NoRegion, true),
            (true, PerPixel, false, NoRegion, true),
            (true, Alpha(255), true, NoRegion, false),
            (true, PerPixel, true, NoRegion, false),
            // Window shape
            (false, NotLayered, false, Inside, true),
            (false, NotLayered, false, Outside, false),
            (false, Alpha(255), false, Inside, true),
            (false, ColorKey, false, Inside, false),
            (true, NotLayered, false, Inside, true),
            (true, NotLayered, false, Outside, false),
        ];
        for (ours, layered, transparent, region, want) in cases {
            assert_eq!(
                hit_accepts(ours, layered, transparent, region),
                want,
                "ours={ours} layered={layered:?} transparent={transparent} region={region:?}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn place_plan_maps_each_argument_to_one_flag() {
        use windows::Win32::UI::WindowsAndMessaging::{
            HWND_TOP, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
            SWP_SHOWWINDOW,
        };
        let (after, flags) = win::place_plan(true, ZOrder::Keep, true, false);
        assert!(after.is_invalid());
        assert_eq!(
            flags,
            SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW | SWP_NOACTIVATE | SWP_NOZORDER
        );
        let (after, flags) = win::place_plan(false, ZOrder::Topmost, false, true);
        assert_eq!(after, HWND_TOPMOST);
        assert_eq!(flags.0, 0);
        let (after, _) = win::place_plan(false, ZOrder::Top, false, false);
        assert_eq!(after, HWND_TOP);
        let (after, flags) =
            win::place_plan(false, ZOrder::After(NativeWindow(0x1234)), false, false);
        assert_eq!(after.0 as isize, 0x1234);
        assert_eq!(flags, SWP_NOACTIVATE);
    }

    // ── real windows — off-screen position · no activation (does not appear on the viewer's screen) ──

    #[cfg(windows)]
    mod real {
        use super::super::*;
        use crate::test_window::TestWindow;
        use windows::core::w;
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, GetWindowRect, IsWindowVisible, SetWindowPos,
            ShowWindow, HWND_TOPMOST, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, SW_SHOWMINNOACTIVE,
            SW_SHOWNOACTIVATE, WINDOW_EX_STYLE, WINDOW_STYLE, WS_EX_TOOLWINDOW, WS_POPUP,
        };

        fn visible(w: &TestWindow) -> bool {
            // SAFETY: a query that only reads a value.
            unsafe { IsWindowVisible(w.hwnd()) }.as_bool()
        }

        fn hide(w: &TestWindow) {
            // The returned value is the previous visibility, not whether it succeeded.
            // SAFETY: a live window held by this test.
            let _ = unsafe { ShowWindow(w.hwnd(), SW_HIDE) };
        }

        /// A borderless tool window with a chosen extended style — [`TestWindow`] cannot choose the extended style. At `rect` it is always shown on top
        /// without activation. Destroyed when dropped.
        struct StyledWindow(HWND);

        impl StyledWindow {
            fn new(ex: WINDOW_EX_STYLE, rect: PhysRect) -> Self {
                // `SS_NOTIFY` (a constant of `Win32_UI_Controls`, which this crate does not turn on) — without it STATIC does not answer the OS hit test
                //   and `WindowFromPoint` passes over that window (measured).
                const SS_NOTIFY: WINDOW_STYLE = WINDOW_STYLE(0x100);
                // SAFETY: only a system class name and value arguments. No instance, parent or menu.
                let hwnd = unsafe {
                    CreateWindowExW(
                        WS_EX_TOOLWINDOW | ex,
                        w!("STATIC"),
                        w!("engram-platform-test"),
                        WS_POPUP | SS_NOTIFY,
                        rect.x,
                        rect.y,
                        rect.width,
                        rect.height,
                        HWND::default(),
                        None,
                        None,
                        None,
                    )
                }
                .expect("시험 창 짓기");
                let w = Self(hwnd);
                // SAFETY: a live window this value built and value arguments.
                unsafe {
                    SetWindowPos(
                        hwnd,
                        HWND_TOPMOST,
                        rect.x,
                        rect.y,
                        rect.width,
                        rect.height,
                        SWP_SHOWWINDOW | SWP_NOACTIVATE,
                    )
                }
                .expect("시험 창 보이기");
                w
            }

            fn native(&self) -> NativeWindow {
                NativeWindow(self.0 .0 as isize)
            }
        }

        impl Drop for StyledWindow {
            fn drop(&mut self) {
                // SAFETY: a window this value built.
                let _ = unsafe { DestroyWindow(self.0) };
            }
        }

        #[test]
        fn native_window_of_reads_the_win32_handle() {
            let w = TestWindow::new(false);
            let n = NativeWindow::of(&w).expect("Win32 핸들");
            assert_eq!(n, NativeWindow(w.hwnd().0 as isize));
        }

        // A hidden window · a vanished window is not the foreground. The true branch measures against the current foreground window without stealing the foreground — if there is no foreground window (service
        //   session) or it changes while measuring, that branch is skipped.
        #[test]
        fn is_foreground_is_true_only_for_the_foreground_window() {
            use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
            let w = TestWindow::new(true);
            assert_eq!(is_foreground(&w), Some(false), "숨은 창");
            let gone = w.handle_only();
            drop(w);
            assert_eq!(is_foreground(&gone), Some(false), "사라진 창");
            // SAFETY: two queries with no arguments.
            let (before, answer, after) = unsafe {
                let before = GetForegroundWindow();
                let answer = is_foreground(&TestWindow::borrowed(before));
                (before, answer, GetForegroundWindow())
            };
            if !before.is_invalid() && before == after {
                assert_eq!(answer, Some(true), "지금 전경 창");
            }
        }

        #[test]
        fn place_window_moves_sizes_and_shows_without_activation() {
            let w = TestWindow::new(true);
            assert!(!visible(&w));
            let rect = PhysRect {
                x: -21000,
                y: -21000,
                width: 300,
                height: 200,
            };
            place_window(&w, Some(rect), ZOrder::Top, true, false).unwrap();
            assert!(visible(&w));
            assert_eq!(w.outer(), rect);
        }

        #[test]
        fn place_window_without_rect_keeps_place_and_size() {
            let w = TestWindow::new(true);
            let rect = PhysRect {
                x: -21500,
                y: -21500,
                width: 320,
                height: 180,
            };
            place_window(&w, Some(rect), ZOrder::Keep, false, false).unwrap();
            assert!(!visible(&w), "show 거짓이면 보이지 않는다");
            place_window(&w, None, ZOrder::Keep, true, false).unwrap();
            assert!(visible(&w));
            assert_eq!(w.outer(), rect);
        }

        #[test]
        fn closed_window_is_no_window() {
            let w = TestWindow::new(true);
            let gone = w.handle_only();
            drop(w);
            assert!(matches!(
                place_window(&gone, None, ZOrder::Keep, true, false),
                Err(WindowError::NoWindow)
            ));
            assert!(matches!(
                restore_placement(&gone, PhysRect::default(), false, false),
                Err(WindowError::NoWindow)
            ));
        }

        #[test]
        fn top_level_at_skips_hidden_and_minimized_windows() {
            let below = TestWindow::new(true);
            let above = TestWindow::new(true);
            let rect = PhysRect {
                x: -23000,
                y: -23000,
                width: 200,
                height: 200,
            };
            place_window(&below, Some(rect), ZOrder::Topmost, true, false).unwrap();
            place_window(&above, Some(rect), ZOrder::Topmost, true, false).unwrap();
            let p = PhysPoint {
                x: -22900,
                y: -22900,
            };
            let (a, b) = (
                NativeWindow::of(&above).unwrap(),
                NativeWindow::of(&below).unwrap(),
            );
            assert_eq!(top_level_at(p), Some(a));
            let right_edge = PhysPoint {
                x: rect.x + rect.width,
                y: p.y,
            };
            assert_eq!(top_level_at(right_edge), None, "오른쪽 변은 밖이다");
            hide(&above);
            assert_eq!(top_level_at(p), Some(b), "숨은 창은 대상이 아니다");
            // SAFETY: a live window held by this test — minimize without activation.
            unsafe {
                let _ = ShowWindow(below.hwnd(), SW_SHOWMINNOACTIVE);
            }
            let mut r = windows::Win32::Foundation::RECT::default();
            // SAFETY: same as above.
            unsafe { GetWindowRect(below.hwnd(), &mut r) }.unwrap();
            let in_icon = PhysPoint {
                x: r.left,
                y: r.top,
            };
            assert_ne!(
                top_level_at(in_icon),
                Some(b),
                "최소화된 창은 대상이 아니다"
            );
            // SAFETY: same as above.
            unsafe {
                let _ = ShowWindow(above.hwnd(), SW_SHOWNOACTIVATE);
            }
            assert_eq!(top_level_at(p), Some(a));
        }

        #[test]
        fn top_level_at_skips_points_outside_the_window_region() {
            use windows::Win32::Graphics::Gdi::{CreateRectRgn, SetWindowRgn};
            let below = TestWindow::new(true);
            let above = TestWindow::new(true);
            let rect = PhysRect {
                x: -25000,
                y: -25000,
                width: 200,
                height: 200,
            };
            place_window(&below, Some(rect), ZOrder::Topmost, true, false).unwrap();
            place_window(&above, Some(rect), ZOrder::Topmost, true, false).unwrap();
            let (a, b) = (
                NativeWindow::of(&above).unwrap(),
                NativeWindow::of(&below).unwrap(),
            );
            assert!(!describe_window(a).unwrap().has_region);
            // SAFETY: a live window held by this test — on success the region belongs to the OS (do not delete it).
            unsafe {
                let rgn = CreateRectRgn(0, 0, 100, 100);
                assert_ne!(SetWindowRgn(above.hwnd(), rgn, false), 0);
            }
            let inside = PhysPoint {
                x: rect.x + 50,
                y: rect.y + 50,
            };
            let outside = PhysPoint {
                x: rect.x + 150,
                y: rect.y + 150,
            };
            assert_eq!(top_level_at(inside), Some(a), "모양 안");
            assert_eq!(top_level_at(outside), Some(b), "모양 밖이면 밑 창");
            let facts = describe_window(a).unwrap();
            assert!(facts.has_region);
            assert_eq!(facts.pid, std::process::id());
            assert!(
                facts.class.eq_ignore_ascii_case("static"),
                "{}",
                facts.class
            );
        }

        #[test]
        fn top_level_at_mirrors_the_region_of_rtl_windows() {
            use windows::Win32::Foundation::POINT;
            use windows::Win32::Graphics::Gdi::{CreateRectRgn, SetWindowRgn};
            use windows::Win32::UI::WindowsAndMessaging::{WindowFromPoint, WS_EX_LAYOUTRTL};
            let rect = PhysRect {
                x: -26000,
                y: -26000,
                width: 200,
                height: 200,
            };
            for (ex, rtl) in [(WINDOW_EX_STYLE(0), false), (WS_EX_LAYOUTRTL, true)] {
                let w = StyledWindow::new(ex, rect);
                // SAFETY: a live window held by this test — on success the region belongs to the OS (do not delete it).
                unsafe {
                    assert_ne!(SetWindowRgn(w.0, CreateRectRgn(0, 0, 100, 200), false), 0);
                }
                assert_eq!(describe_window(w.native()).unwrap().rtl, rtl);
                // Shape x [0, 100) — with left layout the left half is that window · with right layout the right half is.
                for dx in [0, 99, 100, 199] {
                    let p = PhysPoint {
                        x: rect.x + dx,
                        y: rect.y + 50,
                    };
                    let want = (dx < 100) != rtl;
                    // SAFETY: a value-argument query.
                    let os_hit = unsafe { WindowFromPoint(POINT { x: p.x, y: p.y }) } == w.0;
                    assert_eq!(os_hit, want, "OS 적중 판정 rtl={rtl} dx={dx}");
                    assert_eq!(
                        top_level_at(p) == Some(w.native()),
                        want,
                        "rtl={rtl} dx={dx}"
                    );
                }
            }
        }

        #[test]
        fn transparent_only_window_still_takes_the_click() {
            use windows::Win32::Foundation::POINT;
            use windows::Win32::UI::WindowsAndMessaging::{WindowFromPoint, WS_EX_TRANSPARENT};
            let rect = PhysRect {
                x: -28000,
                y: -28000,
                width: 120,
                height: 90,
            };
            let w = StyledWindow::new(WS_EX_TRANSPARENT, rect);
            let p = PhysPoint {
                x: rect.x + 10,
                y: rect.y + 10,
            };
            // SAFETY: a value-argument query.
            let os_hit = unsafe { WindowFromPoint(POINT { x: p.x, y: p.y }) } == w.0;
            assert!(os_hit, "OS 적중 판정");
            assert_eq!(top_level_at(p), Some(w.native()));
        }

        #[test]
        fn describe_window_reads_layered_attributes() {
            use windows::Win32::Foundation::COLORREF;
            use windows::Win32::UI::WindowsAndMessaging::{
                SetLayeredWindowAttributes, LAYERED_WINDOW_ATTRIBUTES_FLAGS, LWA_ALPHA,
                LWA_COLORKEY, WS_EX_LAYERED,
            };
            let rect = PhysRect {
                x: -27000,
                y: -27000,
                width: 120,
                height: 90,
            };
            let plain = StyledWindow::new(WINDOW_EX_STYLE(0), rect);
            assert_eq!(
                describe_window(plain.native()).unwrap().layered,
                LayeredKind::NotLayered
            );
            drop(plain);
            let w = StyledWindow::new(WS_EX_LAYERED, rect);
            assert_eq!(
                describe_window(w.native()).unwrap().layered,
                LayeredKind::PerPixel,
                "속성을 아직 안 준 layered 창"
            );
            let cases = [
                (LWA_ALPHA, 0, LayeredKind::Alpha(0)),
                (LWA_ALPHA, 128, LayeredKind::Alpha(128)),
                (
                    LAYERED_WINDOW_ATTRIBUTES_FLAGS(0),
                    0,
                    LayeredKind::Alpha(255),
                ),
                (LWA_COLORKEY, 255, LayeredKind::ColorKey),
                (LWA_ALPHA | LWA_COLORKEY, 0, LayeredKind::ColorKey),
            ];
            for (flags, alpha, want) in cases {
                // SAFETY: a live layered window held by this test and value arguments.
                unsafe { SetLayeredWindowAttributes(w.0, COLORREF(0), alpha, flags) }
                    .expect("layered 속성");
                assert_eq!(
                    describe_window(w.native()).unwrap().layered,
                    want,
                    "flags={flags:?} alpha={alpha}"
                );
            }
            // Our window is skipped only if click-through — a color-key window is also an answer (`hit_accepts`).
            let p = PhysPoint {
                x: rect.x + 10,
                y: rect.y + 10,
            };
            assert_eq!(top_level_at(p), Some(w.native()));
        }

        /// It shows an ordinary (non-tool) window on screen, so it briefly appears on the viewer's screen — run it by hand in a desktop session.
        #[test]
        #[ignore = "화면 안에 창을 잠깐 보인다 — 데스크톱 세션에서 손으로"]
        fn restore_placement_round_trips_screen_coordinates() {
            let w = TestWindow::new(false);
            let normal = PhysRect {
                x: 200,
                y: 200,
                width: 400,
                height: 300,
            };
            restore_placement(&w, normal, false, false).unwrap();
            assert!(visible(&w));
            assert_eq!(w.outer(), normal);
            hide(&w);
        }
    }
}
