//! 실 Win32 창 시험 도우미 — 이 crate 의 `window` · `move_loop` 시험만 쓴다(부르는 쪽 crate 는 Tauri 창을 쓴다).
//!
//! 시스템 창 클래스 `STATIC` 로 숨긴 채 화면 밖에 짓는다 — 클래스를 등록하지 않아 시험끼리 겹칠 것이 없고, 창은
//! 지은 스레드에 묶이므로 병렬 시험이 저마다 제 창을 쓴다. 버리면 아직 있는 창을 부순다.

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
    /// `tool` = 참이면 테두리 없는 도구 창(작업 표시줄 · Alt+Tab 에 안 나온다) · 거짓이면 보통 최상위 창.
    pub(crate) fn new(tool: bool) -> Self {
        let (ex, style) = if tool {
            (WS_EX_TOOLWINDOW, WS_POPUP)
        } else {
            (WS_EX_APPWINDOW, WS_OVERLAPPEDWINDOW)
        };
        // SAFETY: 시스템 클래스 이름 · 값 인자뿐이다. 인스턴스 · 부모 · 메뉴 없음.
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
        // SAFETY: 이 값이 쥔 창 — 사라졌으면 실패한다.
        unsafe { GetWindowRect(self.hwnd, &mut r) }.expect("창 사각형");
        PhysRect {
            x: r.left,
            y: r.top,
            width: r.right - r.left,
            height: r.bottom - r.top,
        }
    }

    /// 같은 창을 가리키되 창을 쥐지 않는 손잡이 — 창이 사라진 뒤의 갈래를 재는 데 쓴다.
    pub(crate) fn handle_only(&self) -> RawHandle {
        RawHandle(self.hwnd.0 as isize)
    }
}

impl Drop for TestWindow {
    fn drop(&mut self) {
        // SAFETY: 이 값이 지은 창 — 시험이 먼저 부쉈을 수 있어 있는지 본 뒤 부순다.
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
    // SAFETY: 시험 창의 핸들 값을 그대로 싣는다 — 쓰는 쪽은 `IsWindow` 로 살아 있는지 본다.
    Ok(unsafe { WindowHandle::borrow_raw(raw) })
}

impl HasWindowHandle for TestWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        borrow(self.hwnd.0 as isize)
    }
}

/// 창을 쥐지 않는 핸들 — [`TestWindow::handle_only`].
pub(crate) struct RawHandle(isize);

impl HasWindowHandle for RawHandle {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        borrow(self.0)
    }
}
