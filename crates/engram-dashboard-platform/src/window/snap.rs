//! 스냅 레이아웃 오버레이 — Windows 11 이 최대화 단추에 커서를 올리면 띄우는 배치 고르기 메뉴를 웹이 그린 최대화 단추에서도
//! 뜨게 한다(TRD S26-window-chrome §8-4). ★cargo 기능 `snap-overlay` 뒤 · 기본 꺼짐★(사용자 결정 Q1 2026-10-24 — 켜지 않는다 ·
//! 코드는 남긴다). 꺼진 채 두는 까닭 = 주석으로 막은 코드는 컴파일되지 않아 썩는다 — 켠 빌드는 CI 가 컴파일한다.
//!
//! 꼴 — 웹 최대화 단추 자리에 시스템 클래스 `STATIC` 자식 창 하나를 맨 위 형제로 얹고, 그 창의 적중 판정(`WM_NCHITTEST`)이
//! `HTMAXBUTTON` 을 답하게 창 프로시저를 갈아 끼운다 — OS 는 그 자리를 최대화 단추로 보고 메뉴를 띄운다(MS 「Support snap
//! layouts for desktop apps on Windows 11」 — 사용자 지정 제목 줄은 최대화 단추 자리에서 `HTMAXBUTTON` 을 돌려준다).
//! - 그 창이 커서를 가져가 웹 단추는 맴돎을 모른다 — 맴돎 바뀜을 부르는 쪽 콜백([`HoverFn`])으로 넘긴다.
//! - 떼기는 그 창이 부모에 최대화 / 원래 크기 시스템 명령(`WM_SYSCOMMAND`)을 보낸다 — 제목 줄 두 번 누름과 같은 OS 길이라
//!   명령 버스를 지나지 않는다. 키보드(Tab · Enter)는 웹 단추가 그대로 받는다.
//! - 웹이 그린 단추가 그대로 보이는 것은 스파이크(2026-10-24 · TRD §1-3 S6)에서 본 것이고 까닭은 재지 않았다.
//!
//! 참고 — `Hyph-M/tauri-plugin-snap-layout`(MIT · `src/snap.rs`)의 오버레이를 옮겨 다시 지었다(코드 복붙이 아니라 꼴 차용).
//! 다른 점: 창 프로시저를 `SetWindowLongPtrW` 로 갈아 끼운다(comctl32 서브클래스 — `Win32_UI_Shell` 바인딩을 들이지 않는다) ·
//! 사각형은 부르는 쪽이 부모 클라이언트 물리 px 로 준다 · 맴돎은 부르는 쪽 콜백으로 간다 · 오버레이를 부모의 자식 ID 로
//! 찾는다(전역 표 없음).
//!
//! ★스레드★ — 두 함수 다 부모 창을 만든 스레드(메인)에서 부른다(자식 창은 부른 스레드에 묶이고, 창 프로시저 · 콜백도 그
//! 스레드에서 돈다). 다른 스레드에서 부르면 아무것도 하지 않고 [`WindowError::Os`] 다 — 부모 창 스레드를 대 보는 가드가
//! 막는다(남의 스레드에서 자식 창을 지으면 부모에 보내는 알림이 그 스레드를 기다려 교착할 수 있고, 지은 창은 부른 스레드가
//! 끝나면 함께 사라진다). ★콜백 안에서 이 모듈 함수를 부르지 말 것★ — 막지는 않지만(콜백을 갈아 끼워도 · 오버레이를
//! 걷어도 도는 콜백은 끝까지 돈다) 그 꼴을 시험하지 않았고, 나감(`WM_NCMOUSELEAVE`)은 콜백 뒤 앞 프로시저로 넘기므로
//! 콜백 안에서 걷으면 부서진 창으로 넘긴다.
//!
//! Windows 밖 = [`WindowError::Unsupported`](이 crate 「들이는 규칙」 3 — 자리채움). 켠 빌드의 동작(배율 · 창마다 ·
//! 포커스)은 켜는 날 다시 잰다(TRD §14 W8).

// ADR-0266

use raw_window_handle::HasWindowHandle;

use super::{PhysRect, WindowError};

/// 맴돎 바뀜 — 참 = 커서가 오버레이에 들어왔다 · 거짓 = 나갔다. 부모 창 스레드에서 불린다. 오버레이를 걷을 때는 부르지
/// 않는다 — 걷는 쪽이 자기 맴돎 상태를 거둔다.
pub type HoverFn = Box<dyn Fn(bool)>;

/// `parent` 의 클라이언트 물리 px 사각형 `rect` 에 오버레이를 놓는다 — 없으면 짓고 있으면 옮긴다(맨 위 형제로 다시 올린다).
/// `on_hover` 는 앞 콜백을 갈아 끼운다. 너비 · 높이가 1 보다 작거나 `parent` 를 만든 스레드가 아닌 스레드에서 부르면
/// 아무것도 하지 않고 [`WindowError::Os`] 다(빈 사각형이면 부르는 쪽은 걷기를 부른다).
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

/// `parent` 의 오버레이를 걷는다 — 없으면 무동작 `Ok`. `parent` 를 만든 스레드가 아닌 스레드에서 부르면 아무것도 하지 않고
/// [`WindowError::Os`] 다.
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

    /// 오버레이의 자식 창 ID — 부모에서 [`find`] 가 이 값으로 찾는다. 같은 ID 의 남의 자식 창은 창 프로시저가 우리 것이
    /// 아니라 걸러진다.
    const OVERLAY_ID: i32 = 0x534E;

    /// 오버레이 창 하나의 상태 — `GWLP_USERDATA` 가 쥐고 `WM_NCDESTROY` 가 푼다.
    struct State {
        /// 갈아 끼우기 전 창 프로시저(`STATIC` 의 것).
        prev: isize,
        parent: HWND,
        hovering: bool,
        /// 콜백을 부르는 동안 복제를 쥐어, 그사이 갈아 끼우거나 걷어도 도는 콜백이 끝까지 산다.
        on_hover: Rc<dyn Fn(bool)>,
    }

    fn os(api: &str, e: windows::core::Error) -> WindowError {
        WindowError::Os(io::Error::other(format!("{api}: {e}")))
    }

    fn proc_addr() -> isize {
        overlay_proc as *const () as usize as isize
    }

    /// `parent` 의 오버레이 — 자식 ID 가 맞고 창 프로시저가 우리 것인 자식.
    pub(super) fn find(parent: HWND) -> Option<HWND> {
        // SAFETY: 값만 보는 조회 둘 — 없으면 `Err` · 0 이다.
        unsafe {
            let child = GetDlgItem(parent, OVERLAY_ID).ok()?;
            (GetWindowLongPtrW(child, GWLP_WNDPROC) == proc_addr()).then_some(child)
        }
    }

    /// 부모 창 스레드 가드(머리 「스레드」) — `parent` 를 만든 스레드가 아니면 아무것도 건드리기 전에 거절한다. 그 뒤의
    /// 상태 읽기 · 쓰기와 창 프로시저가 같은 스레드라 겹치지 않는다는 것도 이 가드가 받친다.
    fn on_parent_thread(parent: HWND) -> Result<(), WindowError> {
        // SAFETY: 값만 보는 조회 둘 — 사라진 창이면 0 이라 맞지 않아 거절된다.
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
                // SAFETY: 우리 오버레이(`find` 가 창 프로시저로 걸렀다)의 상태 — 이 스레드의 창이라 창 프로시저와 겹치지 않는다.
                //   도는 콜백은 자기 복제를 쥐고 있어 여기서 옛 것을 버려도 된다.
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
        // SAFETY: 살아 있는 자식 창과 값 인자뿐이다. 맨 위 형제로 올려 웹뷰 자식 창 위에 둔다.
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
        // SAFETY: 시스템 클래스 이름 · 값 인자 · 살아 있는 부모(`hwnd_of` 가 걸렀다). 자식 ID 는 `HMENU` 자리로 넘긴다.
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
        // SAFETY: 방금 지은 이 스레드의 창 — 상태를 먼저 싣고 창 프로시저를 갈아 끼운다(갈아 끼운 프로시저는 상태가 있다고
        //   믿는다). 상태는 `WM_NCDESTROY` 가 푼다.
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
        // SAFETY: 이 스레드의 우리 오버레이 — 부수면 `WM_NCDESTROY` 가 상태를 푼다.
        unsafe { DestroyWindow(child) }.map_err(|e| os("DestroyWindow", e))
    }

    /// 맴돎이 끝나면(커서가 비클라이언트 자리를 떠나면) `WM_NCMOUSELEAVE` 를 받게 한다. 실패하면 나감을 못 듣는다 — 다음
    /// 적중 판정까지 맴돎이 남는다.
    fn track_leave(hwnd: HWND) {
        let mut tme = TRACKMOUSEEVENT {
            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE | TME_NONCLIENT,
            hwndTrack: hwnd,
            dwHoverTime: 0,
        };
        // SAFETY: 지역 구조체 하나를 넘긴다.
        let _ = unsafe { TrackMouseEvent(&mut tme) };
    }

    /// 맴돎 칸을 `hover` 로 바꾸고, 바뀌었으면 부를 콜백을 돌려준다 — 상태의 빌림은 콜백을 부르기 전에 끝난다.
    ///
    /// # Safety
    /// `state` 는 살아 있는 이 창의 상태다.
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
            // 상태를 싣기 전 · 푼 뒤 — 갈아 끼우기 전 프로시저를 모르니 기본 처리로 넘긴다.
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
            // 우리 몫(맴돎 칸 · 콜백)만 하고 앞 프로시저로 넘긴다 — 스파이크가 그렇게 했고 그 빌드에서 메뉴가 떴다(TRD §1-3
            //   S6 — 사용자 관측). 삼키는 꼴은 화면에서 잰 적이 없다. `prev` 는 위에서 이미 복사해 두어 상태를 다시 읽지
            //   않는다 — 단 콜백이 오버레이를 걷으면 부서진 창으로 넘기게 되므로 콜백 안에서 걷지 않는다(머리 「스레드」).
            WM_NCMOUSELEAVE => {
                if let Some(f) = flip(state, false) {
                    f(false);
                }
            }
            // 누름은 삼킨다 — 기본 처리가 OS 캡션 단추 누름 추적을 시작하지 않게.
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
            // Windows 캡션 단추는 화살표 커서다(웹 단추의 `cursor: default` 와 같다).
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

// 실 창 — 화면 밖 · 숨긴 부모(`test_window`)에 오버레이를 짓는다. 메시지는 같은 스레드의 `SendMessageW` 로 창 프로시저를
//   곧바로 부르고, 부모에 보낸 시스템 명령은 꺼내기만 해(`PeekMessageW` · `PM_REMOVE`) 창이 실제로 커지지 않는다.
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
        // SAFETY: 살아 있는 창이면 채우고, 아니면 실패한다.
        unsafe { GetWindowRect(hwnd, &mut r) }.expect("창 사각형");
        PhysRect {
            x: r.left,
            y: r.top,
            width: r.right - r.left,
            height: r.bottom - r.top,
        }
    }

    /// 오버레이의 부모 클라이언트 자리 — 부모가 틀 없는 창(`WS_POPUP`)이라 바깥 사각형의 왼쪽 위가 클라이언트 원점이다.
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
        // SAFETY: 이 스레드의 창 — 창 프로시저를 곧바로 부른다.
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
        // SAFETY: 값만 보는 조회.
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
            // SAFETY: 이 스레드 큐에서 부모에 온 시스템 명령만 꺼낸다 — 내보내지 않아 창이 커지지 않는다.
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
        // 가드가 빠지면 남의 스레드가 이 창에 메시지를 보내고 답을 기다린다 — 그동안 이 스레드가 받은 메시지를 처리해
        //   교착 대신 아래 단언의 실패로 끝나게 한다(`PeekMessageW` 가 들여다보는 동안 보내온 메시지를 처리한다).
        let deadline = Instant::now() + Duration::from_secs(5);
        while !worker.is_finished() {
            assert!(Instant::now() < deadline, "남의 스레드가 끝나지 않았다");
            let mut msg = MSG::default();
            // SAFETY: 이 스레드 큐를 들여다보기만 한다(꺼내지 않는다).
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
