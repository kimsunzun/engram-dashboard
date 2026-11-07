//! Helper for real Win32 window tests — used only by this crate's `window` tests (the calling crate uses a Tauri window).
//!
//! Built hidden and off-screen with the system window class `STATIC` — no class is registered, so tests have nothing to overlap, and a window
//! is bound to the thread that built it, so parallel tests each use their own window. Dropping it destroys the window if it is still there.

use std::num::NonZeroIsize;

use raw_window_handle::{
    HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle,
};
use windows::core::w;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetWindowRect, IsWindow, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
    WS_OVERLAPPEDWINDOW, WS_POPUP,
};

use crate::window::PhysRect;

pub(crate) struct TestWindow {
    hwnd: HWND,
}

impl TestWindow {
    /// `tool` = if true, a borderless tool window (does not appear in the taskbar or Alt+Tab) · if false, an ordinary top-level window.
    pub(crate) fn new(tool: bool) -> Self {
        let (ex, style) = if tool {
            (WS_EX_TOOLWINDOW, WS_POPUP)
        } else {
            (WS_EX_APPWINDOW, WS_OVERLAPPEDWINDOW)
        };
        // SAFETY: only a system class name and value arguments. No instance, parent or menu.
        let hwnd = unsafe {
            CreateWindowExW(
                ex,
                w!("STATIC"),
                w!("engram-platform-test"),
                style,
                -30000,
                -30000,
                120,
                90,
                HWND::default(),
                None,
                None,
                None,
            )
        }
        .expect("시험 창 짓기");
        Self { hwnd }
    }

    pub(crate) fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub(crate) fn outer(&self) -> PhysRect {
        let mut r = RECT::default();
        // SAFETY: the window this value holds — it fails if it is gone.
        unsafe { GetWindowRect(self.hwnd, &mut r) }.expect("창 사각형");
        PhysRect {
            x: r.left,
            y: r.top,
            width: r.right - r.left,
            height: r.bottom - r.top,
        }
    }

    /// A handle that points at the same window but does not hold it — used to measure the branch after the window is gone.
    pub(crate) fn handle_only(&self) -> RawHandle {
        RawHandle(self.hwnd.0 as isize)
    }

    /// A handle that points at a window this test did not build (including another process's window) without holding it.
    pub(crate) fn borrowed(hwnd: HWND) -> RawHandle {
        RawHandle(hwnd.0 as isize)
    }
}

impl Drop for TestWindow {
    fn drop(&mut self) {
        // SAFETY: the window this value built — the test may have destroyed it first, so check it exists before destroying.
        unsafe {
            if IsWindow(self.hwnd).as_bool() {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}

fn borrow(hwnd: isize) -> Result<WindowHandle<'static>, HandleError> {
    let raw = RawWindowHandle::Win32(Win32WindowHandle::new(
        NonZeroIsize::new(hwnd).ok_or(HandleError::Unavailable)?,
    ));
    // SAFETY: carries the test window's handle value as is — the user checks with `IsWindow` whether it is alive.
    Ok(unsafe { WindowHandle::borrow_raw(raw) })
}

impl HasWindowHandle for TestWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        borrow(self.hwnd.0 as isize)
    }
}

/// A handle that does not hold the window — [`TestWindow::handle_only`].
pub(crate) struct RawHandle(isize);

impl HasWindowHandle for RawHandle {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        borrow(self.0)
    }
}
