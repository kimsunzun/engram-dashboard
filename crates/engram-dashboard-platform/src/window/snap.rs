// ADR-0304 — snap overlay feature
//! Snap layout overlay — makes the layout-picker menu that Windows 11 shows when the cursor is over the maximize button also
//! appear on a web-drawn maximize button (TRD S26-window-chrome §8-4). ★Behind the cargo feature `snap-overlay` · off by default★ (user decision Q1 2026-10-24 — we do not turn it on ·
//! we keep the code). Why it stays off = code blocked by comments is not compiled and rots — a build with it on is compiled by CI.
//!
//! Design — over the web maximize button's spot, one system-class `STATIC` child window is laid as the topmost sibling, and its window procedure is swapped so that its hit test (`WM_NCHITTEST`)
//! answers `HTMAXBUTTON` — the OS sees that spot as the maximize button and shows the menu (MS 「Support snap
//! layouts for desktop apps on Windows 11」 — a custom title bar returns `HTMAXBUTTON` at the maximize button's spot).
//! - That window takes the cursor, so the web button does not know about hover — the hover change is handed to the caller's callback ([`HoverFn`]).
//! - On release, that window sends a maximize / restore system command (`WM_SYSCOMMAND`) to the parent — the same OS path as double-clicking the title bar, so
//!   it does not pass through the command bus. The keyboard (Tab · Enter) is still received by the web button.
//! - That the web-drawn button stays visible was seen in the spike (2026-10-24 · TRD §1-3 S6) and the reason was not measured.
//!
//! Reference — rebuilt by porting the overlay of `Hyph-M/tauri-plugin-snap-layout` (MIT · `src/snap.rs`) (borrowing the shape, not copy-pasting code).
//! Differences: the window procedure is swapped with `SetWindowLongPtrW` (not a comctl32 subclass — no `Win32_UI_Shell` binding is brought in) ·
//! the rectangle is given by the caller in parent-client physical px · hover goes to the caller's callback · the overlay is found
//! by its child ID under the parent (no global table).
//!
//! ★Threads★ — both functions are called from the thread that created the parent window (main) (a child window is bound to the calling thread, and the window procedure and callback also run on that
//! thread). Called from another thread, they do nothing and give [`WindowError::Os`] — a guard that checks the parent window's thread
//! stops it (building a child window on someone else's thread can deadlock because notifications sent to the parent wait on that thread, and the built window vanishes when the calling thread
//! ends). ★Do not call this module's functions inside the callback★ — it is not blocked (even if the callback is swapped or the overlay
//! removed, a running callback runs to the end) but that pattern was not tested, and leave (`WM_NCMOUSELEAVE`) is passed to the previous procedure after the callback, so
//! removing inside the callback passes it to a destroyed window.
//!
//! Outside Windows = [`WindowError::Unsupported`] (this crate's 「들이는 규칙」 3 — placeholder). The behavior of the build with it on (scaling · per window ·
//! focus) is measured again on the day it is turned on (TRD §14 W8).

// ADR-0266

use raw_window_handle::HasWindowHandle;

use super::{PhysRect, WindowError};

/// Hover change — true = the cursor entered the overlay · false = it left. Called on the parent window's thread. It is not called when
/// removing the overlay — the remover clears its own hover state.
pub type HoverFn = Box<dyn Fn(bool)>;

/// Places the overlay at `rect`, the client physical-px rectangle of `parent` — builds it if absent, moves it if present (raises it again as the topmost sibling).
/// `on_hover` replaces the previous callback. If the width or height is below 1 or it is called from a thread other than the one that created `parent`,
/// it does nothing and gives [`WindowError::Os`] (for an empty rectangle the caller calls remove).
pub fn place_overlay(
    parent: &impl HasWindowHandle,
    rect: PhysRect,
    on_hover: HoverFn,
) -> Result<(), WindowError> {
    #[cfg(windows)]
    {
        win::place_overlay(parent, rect, on_hover)
    }
    #[cfg(not(windows))]
    {
        let _ = (parent, rect, on_hover);
        Err(WindowError::Unsupported)
    }
}

/// Removes the overlay of `parent` — a no-op `Ok` if there is none. Called from a thread other than the one that created `parent`, it does nothing and gives
/// [`WindowError::Os`].
pub fn remove_overlay(parent: &impl HasWindowHandle) -> Result<(), WindowError> {
    #[cfg(windows)]
    {
        win::remove_overlay(parent)
    }
    #[cfg(not(windows))]
    {
        let _ = parent;
        Err(WindowError::Unsupported)
    }
}

#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    use std::io;
    use std::rc::Rc;

    use raw_window_handle::HasWindowHandle;
    use windows::core::w;
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        TrackMouseEvent, TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CallWindowProcW, CreateWindowExW, DefWindowProcW, DestroyWindow, GetDlgItem,
        GetWindowLongPtrW, GetWindowThreadProcessId, IsZoomed, LoadCursorW, PostMessageW,
        SetCursor, SetWindowLongPtrW, SetWindowPos, GWLP_USERDATA, GWLP_WNDPROC, HMENU,
        HTMAXBUTTON, HWND_TOP, IDC_ARROW, SC_MAXIMIZE, SC_RESTORE, SWP_NOACTIVATE, SWP_SHOWWINDOW,
        WINDOW_EX_STYLE, WM_NCDESTROY, WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN,
        WM_NCLBUTTONUP, WM_NCMOUSELEAVE, WM_SETCURSOR, WM_SYSCOMMAND, WNDPROC, WS_CHILD,
        WS_CLIPSIBLINGS, WS_VISIBLE,
    };

    use super::super::win::hwnd_of;
    use super::{HoverFn, PhysRect, WindowError};

    /// The overlay's child window ID — [`find`] looks for it by this value under the parent. Someone else's child window with the same ID is filtered out because its window procedure is
    /// not ours.
    const OVERLAY_ID: i32 = 0x534E;

    /// The state of one overlay window — `GWLP_USERDATA` holds it and `WM_NCDESTROY` releases it.
    struct State {
        /// The window procedure before the swap (`STATIC`'s).
        prev: isize,
        parent: HWND,
        hovering: bool,
        /// A clone is held while the callback runs, so even if it is swapped or removed meanwhile the running callback lives to the end.
        on_hover: Rc<dyn Fn(bool)>,
    }

    fn os(api: &str, e: windows::core::Error) -> WindowError {
        WindowError::Os(io::Error::other(format!("{api}: {e}")))
    }

    fn proc_addr() -> isize {
        overlay_proc as *const () as usize as isize
    }

    /// The overlay of `parent` — the child whose child ID matches and whose window procedure is ours.
    pub(super) fn find(parent: HWND) -> Option<HWND> {
        // SAFETY: two queries that only read values — if absent, `Err` · 0.
        unsafe {
            let child = GetDlgItem(parent, OVERLAY_ID).ok()?;
            (GetWindowLongPtrW(child, GWLP_WNDPROC) == proc_addr()).then_some(child)
        }
    }

    /// Parent-window thread guard (header 「스레드」) — if it is not the thread that created `parent`, it refuses before touching anything. This guard also backs that the later
    /// state reads and writes and the window procedure are on the same thread and so do not overlap.
    fn on_parent_thread(parent: HWND) -> Result<(), WindowError> {
        // SAFETY: two queries that only read values — if the window vanished it is 0, which does not match, so it is refused.
        let (owner, caller) =
            unsafe { (GetWindowThreadProcessId(parent, None), GetCurrentThreadId()) };
        if owner == caller {
            Ok(())
        } else {
            Err(WindowError::Os(io::Error::other(format!(
                "오버레이는 부모 창 스레드에서만 다룬다: 부모 창 스레드 {owner} · 부른 스레드 {caller}"
            ))))
        }
    }

    pub(super) fn place_overlay(
        parent: &impl HasWindowHandle,
        r: PhysRect,
        on_hover: HoverFn,
    ) -> Result<(), WindowError> {
        if r.width < 1 || r.height < 1 {
            return Err(WindowError::Os(io::Error::other(format!(
                "오버레이 크기가 비었다: {}x{}",
                r.width, r.height
            ))));
        }
        let parent = hwnd_of(parent)?;
        on_parent_thread(parent)?;
        let on_hover: Rc<dyn Fn(bool)> = Rc::from(on_hover);
        let child = match find(parent) {
            Some(child) => {
                // SAFETY: the state of our overlay (filtered by `find` through the window procedure) — it is a window of this thread, so it does not overlap with the window procedure.
                //   A running callback holds its own clone, so it is fine to drop the old one here.
                unsafe {
                    let state = GetWindowLongPtrW(child, GWLP_USERDATA) as *mut State;
                    if !state.is_null() {
                        (*state).on_hover = on_hover;
                    }
                }
                child
            }
            None => create(parent, r, on_hover)?,
        };
        // SAFETY: only a live child window and value arguments. Raise it as the topmost sibling to put it over the webview child window.
        unsafe {
            SetWindowPos(
                child,
                HWND_TOP,
                r.x,
                r.y,
                r.width,
                r.height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )
        }
        .map_err(|e| os("SetWindowPos", e))
    }

    fn create(parent: HWND, r: PhysRect, on_hover: Rc<dyn Fn(bool)>) -> Result<HWND, WindowError> {
        // SAFETY: a system class name · value arguments · a live parent (filtered by `hwnd_of`). The child ID is passed in the `HMENU` slot.
        let child = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!(""),
                WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
                r.x,
                r.y,
                r.width,
                r.height,
                parent,
                HMENU(OVERLAY_ID as isize as *mut c_void),
                HINSTANCE::default(),
                None,
            )
        }
        .map_err(|e| os("CreateWindowExW", e))?;
        // SAFETY: a window of this thread just built — load the state first and then swap the window procedure (the swapped procedure trusts that state
        //   exists). The state is released by `WM_NCDESTROY`.
        unsafe {
            let prev = GetWindowLongPtrW(child, GWLP_WNDPROC);
            let state = Box::new(State {
                prev,
                parent,
                hovering: false,
                on_hover,
            });
            SetWindowLongPtrW(child, GWLP_USERDATA, Box::into_raw(state) as isize);
            SetWindowLongPtrW(child, GWLP_WNDPROC, proc_addr());
        }
        Ok(child)
    }

    pub(super) fn remove_overlay(parent: &impl HasWindowHandle) -> Result<(), WindowError> {
        let parent = hwnd_of(parent)?;
        on_parent_thread(parent)?;
        let Some(child) = find(parent) else {
            return Ok(());
        };
        // SAFETY: our overlay on this thread — destroying it makes `WM_NCDESTROY` release the state.
        unsafe { DestroyWindow(child) }.map_err(|e| os("DestroyWindow", e))
    }

    /// Makes it receive `WM_NCMOUSELEAVE` when hover ends (the cursor leaves the non-client area). If it fails, leaving goes unheard — hover stays until the next
    /// hit test.
    fn track_leave(hwnd: HWND) {
        let mut tme = TRACKMOUSEEVENT {
            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE | TME_NONCLIENT,
            hwndTrack: hwnd,
            dwHoverTime: 0,
        };
        // SAFETY: passes one local struct.
        let _ = unsafe { TrackMouseEvent(&mut tme) };
    }

    /// Changes the hover field to `hover` and, if it changed, returns the callback to call — the borrow of the state ends before the callback is called.
    ///
    /// # Safety
    /// `state` is the state of this live window.
    unsafe fn flip(state: *mut State, hover: bool) -> Option<Rc<dyn Fn(bool)>> {
        let s = &mut *state;
        if s.hovering == hover {
            return None;
        }
        s.hovering = hover;
        Some(Rc::clone(&s.on_hover))
    }

    unsafe extern "system" fn overlay_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
        if state.is_null() {
            // Before the state is loaded · after it is released — the procedure before the swap is unknown, so pass to default handling.
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        let prev: WNDPROC = std::mem::transmute::<isize, WNDPROC>((*state).prev);
        match msg {
            WM_NCHITTEST => {
                if let Some(f) = flip(state, true) {
                    track_leave(hwnd);
                    f(true);
                }
                return LRESULT(HTMAXBUTTON as isize);
            }
            // Do only our share (hover field · callback) and pass on to the previous procedure — the spike did so and the menu appeared in that build (TRD §1-3
            //   S6 — user observation). The variant that swallows the message (not passing it on) has never been tested on screen. `prev` was already copied above so the state is not read again
            //   — but if the callback removes the overlay it would pass to a destroyed window, so do not remove inside the callback (header 「스레드」).
            WM_NCMOUSELEAVE => {
                if let Some(f) = flip(state, false) {
                    f(false);
                }
            }
            // Swallow the press — so default handling does not start tracking an OS caption button press.
            WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK if wparam.0 == HTMAXBUTTON as usize => {
                return LRESULT(0);
            }
            WM_NCLBUTTONUP if wparam.0 == HTMAXBUTTON as usize => {
                let parent = (*state).parent;
                let cmd = if IsZoomed(parent).as_bool() {
                    SC_RESTORE
                } else {
                    SC_MAXIMIZE
                };
                let _ = PostMessageW(parent, WM_SYSCOMMAND, WPARAM(cmd as usize), LPARAM(0));
                return LRESULT(0);
            }
            // The Windows caption button is an arrow cursor (same as the web button's `cursor: default`).
            WM_SETCURSOR => {
                if let Ok(c) = LoadCursorW(HINSTANCE::default(), IDC_ARROW) {
                    SetCursor(c);
                }
                return LRESULT(1);
            }
            WM_NCDESTROY => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(state));
                return CallWindowProcW(prev, hwnd, msg, wparam, lparam);
            }
            _ => {}
        }
        CallWindowProcW(prev, hwnd, msg, wparam, lparam)
    }
}

// Real window — builds the overlay on an off-screen, hidden parent (`test_window`). Messages call the window procedure directly with `SendMessageW`
//   on the same thread, and the system command sent to the parent is only taken out (`PeekMessageW` · `PM_REMOVE`) so the window does not actually grow.
#[cfg(all(test, windows))]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowRect, IsWindow, PeekMessageW, SendMessageW, HTMAXBUTTON, MSG, PM_NOREMOVE,
        PM_REMOVE, SC_MAXIMIZE, WM_NCHITTEST, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP, WM_NCMOUSELEAVE,
        WM_SYSCOMMAND,
    };

    use super::win::find;
    use super::*;
    use crate::test_window::TestWindow;

    const RECT_A: PhysRect = PhysRect {
        x: 10,
        y: 5,
        width: 46,
        height: 30,
    };

    fn rect_of(hwnd: HWND) -> PhysRect {
        let mut r = RECT::default();
        // SAFETY: fills it in if the window is live, otherwise fails.
        unsafe { GetWindowRect(hwnd, &mut r) }.expect("창 사각형");
        PhysRect {
            x: r.left,
            y: r.top,
            width: r.right - r.left,
            height: r.bottom - r.top,
        }
    }

    /// The overlay's parent-client position — the parent is a frameless window (`WS_POPUP`), so the top left of the outer rectangle is the client origin.
    fn placed(parent: &TestWindow, child: HWND) -> PhysRect {
        let p = parent.outer();
        let c = rect_of(child);
        PhysRect {
            x: c.x - p.x,
            y: c.y - p.y,
            ..c
        }
    }

    fn recorder() -> (Rc<RefCell<Vec<bool>>>, HoverFn) {
        let log = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&log);
        (log, Box::new(move |hover| sink.borrow_mut().push(hover)))
    }

    fn send(child: HWND, msg: u32, wparam: usize) -> isize {
        // SAFETY: a window of this thread — calls the window procedure directly.
        unsafe { SendMessageW(child, msg, WPARAM(wparam), LPARAM(0)) }.0
    }

    #[test]
    fn place_moves_the_same_child_and_remove_destroys_it() {
        let parent = TestWindow::new(true);
        place_overlay(&parent, RECT_A, Box::new(|_| {})).expect("놓기");
        let child = find(parent.hwnd()).expect("오버레이가 있다");
        assert_eq!(placed(&parent, child), RECT_A);

        let moved = PhysRect { x: 64, ..RECT_A };
        place_overlay(&parent, moved, Box::new(|_| {})).expect("옮기기");
        assert_eq!(find(parent.hwnd()), Some(child), "같은 자식을 옮긴다");
        assert_eq!(placed(&parent, child), moved);

        remove_overlay(&parent).expect("걷기");
        // SAFETY: a query that only reads a value.
        assert!(!unsafe { IsWindow(child) }.as_bool());
        assert_eq!(find(parent.hwnd()), None);
        remove_overlay(&parent).expect("없을 때 걷기 = 무동작");
    }

    #[test]
    fn empty_rect_is_refused() {
        let parent = TestWindow::new(true);
        let empty = PhysRect { width: 0, ..RECT_A };
        assert!(matches!(
            place_overlay(&parent, empty, Box::new(|_| {})),
            Err(WindowError::Os(_))
        ));
        assert_eq!(find(parent.hwnd()), None);
    }

    #[test]
    fn hit_test_answers_max_button_and_reports_hover_once_per_change() {
        let parent = TestWindow::new(true);
        let (log, on_hover) = recorder();
        place_overlay(&parent, RECT_A, on_hover).expect("놓기");
        let child = find(parent.hwnd()).expect("오버레이");

        assert_eq!(send(child, WM_NCHITTEST, 0), HTMAXBUTTON as isize);
        assert_eq!(send(child, WM_NCHITTEST, 0), HTMAXBUTTON as isize);
        assert_eq!(*log.borrow(), [true], "들어옴은 바뀔 때 한 번");
        send(child, WM_NCMOUSELEAVE, 0);
        assert_eq!(*log.borrow(), [true, false]);
    }

    #[test]
    fn placing_again_replaces_the_hover_callback() {
        let parent = TestWindow::new(true);
        let (first, a) = recorder();
        let (second, b) = recorder();
        place_overlay(&parent, RECT_A, a).expect("놓기");
        place_overlay(&parent, RECT_A, b).expect("다시 놓기");
        let child = find(parent.hwnd()).expect("오버레이");
        send(child, WM_NCHITTEST, 0);
        assert!(first.borrow().is_empty());
        assert_eq!(*second.borrow(), [true]);
    }

    #[test]
    fn release_posts_maximize_to_the_parent_and_press_does_not() {
        let parent = TestWindow::new(true);
        place_overlay(&parent, RECT_A, Box::new(|_| {})).expect("놓기");
        let child = find(parent.hwnd()).expect("오버레이");
        let take = || {
            let mut msg = MSG::default();
            // SAFETY: takes out only the system command that came to the parent in this thread's queue — it is not dispatched, so the window does not grow.
            unsafe {
                PeekMessageW(
                    &mut msg,
                    parent.hwnd(),
                    WM_SYSCOMMAND,
                    WM_SYSCOMMAND,
                    PM_REMOVE,
                )
            }
            .as_bool()
            .then_some(msg.wParam.0)
        };

        assert_eq!(send(child, WM_NCLBUTTONDOWN, HTMAXBUTTON as usize), 0);
        assert_eq!(take(), None, "누름은 아무것도 보내지 않는다");
        assert_eq!(send(child, WM_NCLBUTTONUP, HTMAXBUTTON as usize), 0);
        assert_eq!(
            take(),
            Some(SC_MAXIMIZE as usize),
            "최대화 아닌 부모 = 최대화"
        );
    }

    #[test]
    fn other_thread_is_refused_and_leaves_the_overlay_as_is() {
        let parent = TestWindow::new(true);
        place_overlay(&parent, RECT_A, Box::new(|_| {})).expect("놓기");
        let child = find(parent.hwnd()).expect("오버레이");
        let handle = parent.handle_only();
        let worker = std::thread::spawn(move || {
            let moved = PhysRect { x: 64, ..RECT_A };
            (
                place_overlay(&handle, moved, Box::new(|_| {})),
                remove_overlay(&handle),
            )
        });
        // If the guard is missing, someone else's thread sends a message to this window and waits for the answer — meanwhile this thread handles the messages it received so that, instead of a
        //   deadlock, it ends in the failure of the assertion below (`PeekMessageW` handles messages sent in while it peeks).
        let deadline = Instant::now() + Duration::from_secs(5);
        while !worker.is_finished() {
            assert!(Instant::now() < deadline, "남의 스레드가 끝나지 않았다");
            let mut msg = MSG::default();
            // SAFETY: only peeks at this thread's queue (does not take anything out).
            let _ = unsafe { PeekMessageW(&mut msg, HWND::default(), 0, 0, PM_NOREMOVE) };
            std::thread::sleep(Duration::from_millis(1));
        }
        let (place, remove) = worker.join().expect("남의 스레드");
        assert!(matches!(place, Err(WindowError::Os(_))), "놓기 거절");
        assert!(matches!(remove, Err(WindowError::Os(_))), "걷기 거절");
        assert_eq!(find(parent.hwnd()), Some(child), "오버레이가 그대로 있다");
        assert_eq!(placed(&parent, child), RECT_A, "옮기지 않았다");
    }

    #[test]
    fn destroying_the_parent_frees_the_overlay_state() {
        let (log, on_hover) = recorder();
        let parent = TestWindow::new(true);
        place_overlay(&parent, RECT_A, on_hover).expect("놓기");
        assert_eq!(Rc::strong_count(&log), 2, "상태가 콜백을 쥐고 있다");
        drop(parent);
        assert_eq!(
            Rc::strong_count(&log),
            1,
            "부모와 함께 부서진 자식이 상태를 풀었다"
        );
    }
}
