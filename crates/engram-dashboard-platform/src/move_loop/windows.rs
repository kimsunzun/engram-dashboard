//! 창 이동 루프의 Windows 갈래 — 창 서브클래스 · 스레드 훅 · 시작 신호 · 취소를 겉(`move_loop`)의 순수 상태
//! `Core` 에 잇는다.
//!
//! - **시작은 이 루프만의 신호로 한다** — [`begin`] 은 `SC_MOVE` 를 올리지 않고, 시스템 전역에서 겹치지 않는 등록
//!   메시지(`RegisterWindowMessageW`)에 루프 번호를 실어 올린다. 그것이 닿으면 Posted 확인(끝내라는 청 · Esc · 뗌)을
//!   하고, 넘길 때만 그 자리에서 진짜 `WM_SYSCOMMAND(SC_MOVE | 2)` 를 보낸다(`in_flight` 표를 세운 채 — 서브클래스는
//!   표가 선 동안 처음 닿는 `WM_SYSCOMMAND` 하나만 이 루프의 것으로 친다). 같은 창에 따로 오는 `SC_MOVE | 2`(웹뷰의
//!   `app-region: drag` 가 같은 값 `0xF012` 를 보낸다 — 스파이크 S6)는 자리 · 시각이 같아도 건드리지 않고 OS 에
//!   넘긴다 — `SC_MOVE` 를 직접 올리면 남의 것과 가를 수 없고, 잘못 집으면 우리가 올린 것이 나중에 추적 없는 루프를
//!   세운다.
//! - **상태는 스레드 지역 칸 하나**(`SLOT`)다 — 한 스레드에 루프 하나. 서브클래스의 덧붙임 값에 루프 번호를 싣고,
//!   창 프로시저 · 훅은 그 번호로 칸의 루프를 찾는다. 칸은 짧게만 빌린다 — 빌린 채 메시지를 보내는 OS 호출(창
//!   숨기기 · 취소 보내기 · `SC_MOVE` 보내기) · 받는 쪽 알림을 부르지 않는다(그 안에서 같은 칸으로 다시 들어온다).
//!   락은 없다.
//! - **깊이 가드**(TRD E10 — tao 0.35.3 `event_loop.rs` `public_window_callback` 의 `recurse_depth` 꼴) — 루프는
//!   우리 서브클래스의 `WM_SYSCOMMAND` 처리 틀 안에서 돌므로, 루프 안 소멸의 `WM_NCDESTROY` 는 그 틀이 살아 있는
//!   동안 온다. 그 창의 메시지마다 틀 깊이를 세고, 소멸은 걷고 알리기만 하며, 칸을 비우는 것은 깊이가 0 으로
//!   돌아온 틀이다. 펼쳐지며 돌아오는 틀은 `Core` 가 이미 끝났다고 답해 두 번째 알림을 내지 않는다.
//! - **알림 줄**(`outbox`) — 받는 쪽 알림 안에서 그 창이 부서지면(들어감 알림 → 받는 쪽 → 창 닫기) 끝 알림이 그
//!   알림 도중에 생긴다. 알림을 줄에 넣고 한 번에 하나씩 내보내, 받는 쪽은 다시 불리지 않고 차례(들어감 → 끝)도
//!   지켜진다. 끝을 냈고 줄이 빈 루프는 다음 [`begin`] 이 칸을 넘겨받을 수 있다 — 끝 알림을 받는 자리에서 새 루프를
//!   올릴 수 있게.
//! - **받는 쪽 패닉은 가두지 않는다**(ADR-0292 — 사용자 결정 2026-10-18) — 운영 패닉 훅이 사유를 로그하고 끝내며,
//!   훅이 없어도 되감기가 OS 가 부르는 이 프로시저 경계(`extern "system"`)를 넘으면 abort 다. 두 길 다 「가두지 못한
//!   패닉 = 끝」 정책과 같다.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ffi::c_void;
use std::io;
use std::sync::OnceLock;
use std::time::Instant;

use windows::core::w;
use windows::Win32::Foundation::{BOOL, FALSE, HINSTANCE, HWND, LPARAM, LRESULT, TRUE, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_TRANSITIONS_FORCEDISABLED};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, VK_ESCAPE};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessagePos, GetWindowThreadProcessId, IsWindow, PostMessageW,
    RegisterWindowMessageW, SendMessageW, SetWindowsHookExW, ShowWindow, UnhookWindowsHookEx,
    HC_ACTION, HHOOK, SC_MOVE, SW_HIDE, WH_KEYBOARD, WH_MOUSE, WM_CANCELMODE, WM_ENTERSIZEMOVE,
    WM_LBUTTONUP, WM_NCDESTROY, WM_SYSCOMMAND,
};

use super::{Arrival, Cancel, Core, LoopEvent, MoveLoopEnd, MoveLoopError, MoveLoopOpts};

/// 서브클래스 식별자 — 같은 창의 다른 서브클래스(tao 따위)와 겹치지 않으면 된다.
const SUBCLASS_ID: usize = 0x454E_4D4C;

/// Chromium 이 보내는 그 값 — `SC_MOVE | HTCAPTION`. 제목 줄을 끄는 것처럼 커서를 따라 옮기는 루프다(키보드로
/// 자리를 고르는 `SC_MOVE` 단독과 다르다 — hwnd_message_handler.cc#951).
const SC_DRAGMOVE: usize = SC_MOVE as usize | 0x0002;

struct Record {
    id: u64,
    hwnd: HWND,
    core: Core,
    mouse_hook: Option<HHOOK>,
    key_hook: Option<HHOOK>,
    subclassed: bool,
    depth: u32,
    /// 시작 신호가 닿아 진짜 `SC_MOVE` 를 보내는 중이다 — 서브클래스는 이것이 선 동안 처음 닿는 `WM_SYSCOMMAND`
    /// 하나만 이 루프의 것으로 친다.
    in_flight: bool,
    /// Esc 훅이 이 창의 DWM 전환을 껐다 — 끝에서 되켠다(소멸 끝은 빼고).
    dwm_disabled: bool,
    callback: Option<Box<dyn FnMut(LoopEvent)>>,
    outbox: VecDeque<LoopEvent>,
    delivering: bool,
    began: Instant,
}

impl Record {
    /// 칸을 비워도 되나 — 끝을 냈고, 걸린 틀 · 내보낼 알림이 없다.
    fn spent(&self) -> bool {
        self.core.is_done() && self.depth == 0 && !self.delivering && self.outbox.is_empty()
    }
}

impl Drop for Record {
    fn drop(&mut self) {
        // 걷지 못한 채 칸이 버려지는 길(스레드 끝의 스레드 지역 정리)에서만 남아 있다 — 정상 끝은 `finish` 가 걷는다.
        for hook in [self.mouse_hook.take(), self.key_hook.take()]
            .into_iter()
            .flatten()
        {
            // SAFETY: 이 루프가 건 훅 — 한 번만 걷는다(`take`).
            if let Err(e) = unsafe { UnhookWindowsHookEx(hook) } {
                tracing::warn!(error = %e, "이동 루프 훅을 걷지 못했다");
            }
        }
    }
}

thread_local! {
    static SLOT: RefCell<Option<Record>> = const { RefCell::new(None) };
    static NEXT_ID: Cell<u64> = const { Cell::new(1) };
}

/// 칸의 루프가 `id` 면 `f` 를 그 위에서 돌린다. 칸을 빌릴 수 없으면(이미 빌린 자리에서 다시 들어옴 — 이 모듈은
/// 그런 길을 만들지 않는다) `None` 이다.
fn with_loop<R>(id: u64, f: impl FnOnce(&mut Record) -> R) -> Option<R> {
    SLOT.try_with(|slot| {
        let mut slot = slot.try_borrow_mut().ok()?;
        match slot.as_mut() {
            Some(r) if r.id == id => Some(f(r)),
            _ => None,
        }
    })
    .ok()
    .flatten()
}

/// 칸의 루프(번호 무관 — 훅은 지금 루프 하나의 것이다).
fn with_current<R>(f: impl FnOnce(&mut Record) -> R) -> Option<R> {
    SLOT.try_with(|slot| slot.try_borrow_mut().ok()?.as_mut().map(f))
        .ok()
        .flatten()
}

fn hwnd_value(hwnd: HWND) -> isize {
    hwnd.0 as isize
}

/// 시작 신호의 메시지 번호 — 등록에 실패하면(0) `None` 이고 그 실행 내내 그렇다.
fn begin_msg() -> Option<u32> {
    static MSG: OnceLock<u32> = OnceLock::new();
    // SAFETY: 고정 문자열 하나를 넘기는 등록.
    let msg = *MSG.get_or_init(|| unsafe {
        RegisterWindowMessageW(w!("engram-dashboard-platform/move-loop-begin"))
    });
    (msg != 0).then_some(msg)
}

/// `GetMessagePos` 값 → 화면 점(낮은 · 높은 16 비트가 부호 있는 x · y — 왼쪽 · 위 모니터는 음수다).
fn decode_pos(pos: u32) -> (i32, i32) {
    (
        (pos & 0xFFFF) as u16 as i16 as i32,
        (pos >> 16) as u16 as i16 as i32,
    )
}

pub(super) fn begin(
    hwnd: HWND,
    opts: MoveLoopOpts,
    on_event: Box<dyn FnMut(LoopEvent)>,
) -> Result<(), MoveLoopError> {
    // SAFETY: 값만 보는 조회 둘.
    let (owner, me) = unsafe { (GetWindowThreadProcessId(hwnd, None), GetCurrentThreadId()) };
    if owner == 0 {
        return Err(MoveLoopError::NoWindow);
    }
    if owner != me {
        return Err(MoveLoopError::WrongThread);
    }
    let Some(msg) = begin_msg() else {
        return Err(MoveLoopError::Os(io::Error::other(
            "RegisterWindowMessageW 실패",
        )));
    };

    let id = NEXT_ID.with(|n| {
        let id = n.get();
        n.set(id + 1);
        id
    });
    let replaced = SLOT.with(|slot| -> Result<Option<Record>, MoveLoopError> {
        let mut slot = slot.try_borrow_mut().map_err(|_| MoveLoopError::Busy)?;
        if let Some(r) = slot.as_ref() {
            // 끝을 냈고 내보낼 알림이 없으면 그 틀이 아직 걸려 있어도 넘겨받는다 — 그 틀은 번호가 달라 이 루프를 안
            // 건드린다.
            if !(r.core.is_done() && r.outbox.is_empty()) {
                return Err(MoveLoopError::Busy);
            }
        }
        Ok(slot.replace(Record {
            id,
            hwnd,
            core: Core::new(opts.hide_on_escape),
            mouse_hook: None,
            key_hook: None,
            subclassed: false,
            depth: 0,
            in_flight: false,
            dwm_disabled: false,
            callback: Some(on_event),
            outbox: VecDeque::new(),
            delivering: false,
            began: Instant::now(),
        }))
    })?;
    drop(replaced);

    // SAFETY: 이 스레드의 살아 있는 창(위에서 확인)에 이 모듈의 프로시저를 단다. 덧붙임 값 = 루프 번호.
    if !unsafe { SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, id as usize) }.as_bool()
    {
        abandon(id);
        return Err(MoveLoopError::Os(io::Error::other(
            "SetWindowSubclass 실패",
        )));
    }
    with_loop(id, |r| r.subclassed = true);

    // 스레드 지역 훅 — Chromium 처럼 마우스 훅이 서야 키 훅을 건다. 마우스 훅을 못 걸면 실패로 치지 않고 끝을
    // `Released` 로 친다(그러지 않으면 모든 끌기가 취소로 읽힌다 — hwnd_message_handler.cc#152-158 · TRD E7).
    // SAFETY: 이 모듈의 훅 프로시저를 지금 스레드에만 건다(모듈 핸들 없음 = 스레드 훅).
    let mouse = unsafe { SetWindowsHookExW(WH_MOUSE, Some(mouse_hook), HINSTANCE::default(), me) };
    let key = match &mouse {
        // SAFETY: 위와 같다.
        Ok(_) => match unsafe {
            SetWindowsHookExW(WH_KEYBOARD, Some(key_hook), HINSTANCE::default(), me)
        } {
            Ok(k) => Some(k),
            Err(e) => {
                tracing::warn!(hwnd = hwnd_value(hwnd), error = %e, "이동 루프 키 훅을 못 걸었다 — Esc 를 못 본다");
                None
            }
        },
        Err(e) => {
            tracing::warn!(hwnd = hwnd_value(hwnd), error = %e, "이동 루프 마우스 훅을 못 걸었다 — 끝을 놓기로 친다");
            None
        }
    };
    let mouse = mouse.ok();
    with_loop(id, |r| {
        r.core.hook_failed = mouse.is_none();
        r.mouse_hook = mouse;
        r.key_hook = key;
    });

    // 웹뷰가 쥔 캡처를 푼다 — 호스트 주 스레드와 웹뷰 UI 스레드가 입력 큐를 나눈다(스파이크 S3). 쥔 캡처가 없으면
    // 할 일이 없을 뿐이라 결과를 보지 않는다.
    // SAFETY: 인자 없는 호출.
    let _ = unsafe { ReleaseCapture() };
    // 캡처를 놓으며 보낸 `WM_CAPTURECHANGED` 의 처리기가 그 창을 부술 수 있다 — 그러면 소멸 끝이 이미 알려졌으므로
    // `Err`(= 알림 없음) 를 돌려주지 않는다.
    if with_loop(id, |r| r.core.is_done()) != Some(false) {
        return Ok(());
    }
    // LPARAM = 마지막 메시지의 커서 자리(Chromium 과 같다) — 진짜 `SC_MOVE` 를 보낼 때 그대로 싣는다.
    // SAFETY: 인자 없는 조회.
    let pos = unsafe { GetMessagePos() };
    // 전제: 이 서브클래스가 이 창에 마지막으로 단 것이다(끌기 시작 때 단다 — tao · wry 는 창을 지을 때) — 위에서 이
    //   신호를 먹을 서브클래스가 없다.
    // SAFETY: 값 인자뿐이다.
    let posted = unsafe { PostMessageW(hwnd, msg, WPARAM(id as usize), LPARAM(pos as isize)) };
    if let Err(e) = posted {
        abandon(id);
        return Err(MoveLoopError::Os(io::Error::other(format!(
            "이동 루프 시작 신호 올리기 실패: {e}"
        ))));
    }
    let (x, y) = decode_pos(pos);
    tracing::debug!(
        hwnd = hwnd_value(hwnd),
        loop_id = id,
        hide_on_escape = opts.hide_on_escape,
        hooks = mouse.is_some(),
        x,
        y,
        cursor = ?crate::input::cursor_pos(),
        "창 이동 루프를 올렸다"
    );
    Ok(())
}

/// 알림 없이 걷는다 — [`begin`] 이 실패를 돌려줄 때만.
fn abandon(id: u64) {
    let parts = with_loop(id, |r| {
        r.core.stage = super::Stage::Done;
        (
            r.hwnd,
            r.mouse_hook.take(),
            r.key_hook.take(),
            std::mem::take(&mut r.subclassed),
        )
    });
    if let Some((hwnd, mouse, key, subclassed)) = parts {
        teardown(hwnd, mouse, key, subclassed);
    }
    let _ = SLOT.try_with(|slot| {
        let taken = slot.try_borrow_mut().ok().and_then(|mut s| {
            if s.as_ref().is_some_and(|r| r.id == id) {
                s.take()
            } else {
                None
            }
        });
        drop(taken);
    });
}

fn teardown(hwnd: HWND, mouse: Option<HHOOK>, key: Option<HHOOK>, subclassed: bool) {
    for hook in [mouse, key].into_iter().flatten() {
        // SAFETY: 이 루프가 건 훅 — 꺼내 온 것이라 한 번만 걷는다.
        if let Err(e) = unsafe { UnhookWindowsHookEx(hook) } {
            tracing::warn!(hwnd = hwnd_value(hwnd), error = %e, "이동 루프 훅을 걷지 못했다");
        }
    }
    // SAFETY: 이 모듈이 단 서브클래스를 같은 프로시저 · 식별자로 걷는다 — 그 프로시저 안에서 불러도 된다(MS 문서 —
    //   `WM_NCDESTROY` 에서 걷는 것이 권장 꼴이다).
    if subclassed
        && !unsafe { RemoveWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID) }.as_bool()
    {
        tracing::warn!(
            hwnd = hwnd_value(hwnd),
            "이동 루프 서브클래스를 걷지 못했다"
        );
    }
}

pub(super) fn end(hwnd: HWND) {
    // SAFETY: 값만 보는 조회 둘.
    let (owner, me) = unsafe { (GetWindowThreadProcessId(hwnd, None), GetCurrentThreadId()) };
    if owner != me {
        // 루프 상태는 그 창 스레드의 것이라 여기서는 볼 수 없다 — 부르는 쪽 계약 위반이지만 끝내지 않는다.
        tracing::debug!(
            hwnd = hwnd_value(hwnd),
            "그 창의 스레드가 아닌 곳의 end_move_loop — 아무것도 안 한다"
        );
        return;
    }
    let cancel = with_current(|r| {
        if r.hwnd == hwnd {
            r.core.on_end_request()
        } else {
            Cancel::None
        }
    })
    .unwrap_or(Cancel::None);
    match cancel {
        Cancel::None => {}
        // SAFETY: 이 스레드의 살아 있는 창(칸에 걸린 루프의 창)에 값 인자 메시지 하나.
        Cancel::Send => unsafe {
            SendMessageW(hwnd, WM_CANCELMODE, WPARAM(0), LPARAM(0));
        },
        Cancel::Post => post_cancel(hwnd),
    }
}

fn post_cancel(hwnd: HWND) {
    // SAFETY: 값 인자 메시지 하나.
    if let Err(e) = unsafe { PostMessageW(hwnd, WM_CANCELMODE, WPARAM(0), LPARAM(0)) } {
        // 돌려줄 길이 없다 — 루프는 사용자가 버튼을 떼면 스스로 끝난다.
        tracing::warn!(hwnd = hwnd_value(hwnd), error = %e, "WM_CANCELMODE 올리기 실패");
    }
}

/// DWM 전환 끄기 · 되켜기 — 숨김 · 보임을 애니메이션 없이 곧바로 하게 하는 꾸밈이라 실패해도 이어 간다.
fn set_transitions_disabled(hwnd: HWND, disabled: bool) -> bool {
    let value: BOOL = if disabled { TRUE } else { FALSE };
    // SAFETY: 살아 있는 지역 BOOL 의 4 바이트를 읽는 호출.
    let set = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_TRANSITIONS_FORCEDISABLED,
            &value as *const BOOL as *const c_void,
            std::mem::size_of::<BOOL>() as u32,
        )
    };
    match set {
        Ok(()) => true,
        Err(e) => {
            tracing::debug!(hwnd = hwnd_value(hwnd), disabled, error = %e, "DWM 전환 설정 실패");
            false
        }
    }
}

/// 끝 — 훅 · 서브클래스를 걷고 끝 알림을 줄에 넣어 내보낸다. 걷기가 알림보다 먼저다(받는 쪽이 그 자리에서 새
/// 루프를 올릴 수 있게).
fn finish(id: u64, end: MoveLoopEnd) {
    let parts = with_loop(id, |r| {
        let hid = r.core.hid();
        r.outbox.push_back(LoopEvent::Ended { end, hid });
        let c = &r.core;
        tracing::debug!(
            hwnd = hwnd_value(r.hwnd),
            loop_id = id,
            ?end,
            esc = c.esc_seen,
            hid,
            mouse_up = c.mouse_up,
            before_entry = matches!(end, MoveLoopEnd::Released { before_entry: true }),
            ms = r.began.elapsed().as_millis() as u64,
            "창 이동 루프 끝"
        );
        (
            r.hwnd,
            r.mouse_hook.take(),
            r.key_hook.take(),
            std::mem::take(&mut r.subclassed),
            std::mem::take(&mut r.dwm_disabled),
        )
    });
    if let Some((hwnd, mouse, key, subclassed, dwm_disabled)) = parts {
        // SAFETY: 값만 보는 조회.
        if dwm_disabled && end != MoveLoopEnd::Destroyed && unsafe { IsWindow(hwnd) }.as_bool() {
            set_transitions_disabled(hwnd, false);
        }
        teardown(hwnd, mouse, key, subclassed);
    }
    deliver(id);
}

/// 줄의 알림을 한 번에 하나씩 받는 쪽에 낸다 — 이미 내보내는 중이면(받는 쪽 안에서 다시 들어옴) 넣기만 하고
/// 돌아간다. 바깥 내보내기가 이어서 낸다.
fn deliver(id: u64) {
    if with_loop(id, |r| std::mem::replace(&mut r.delivering, true)) != Some(false) {
        return;
    }
    loop {
        let next = with_loop(id, |r| {
            let ev = r.outbox.pop_front()?;
            Some((ev, r.callback.take()))
        })
        .flatten();
        let Some((ev, callback)) = next else { break };
        let Some(mut callback) = callback else {
            continue;
        };
        callback(ev);
        // 끝 알림 안에서 새 루프가 칸을 넘겨받았으면 이 루프는 이미 없다 — 받는 쪽은 여기서 버린다.
        let _ = with_loop(id, |r| r.callback = Some(callback));
    }
    with_loop(id, |r| r.delivering = false);
}

fn enter_frame(id: u64) -> bool {
    with_loop(id, |r| r.depth += 1).is_some()
}

fn leave_frame(id: u64) {
    let spent = SLOT
        .try_with(|slot| {
            let mut slot = slot.try_borrow_mut().ok()?;
            let r = slot.as_mut().filter(|r| r.id == id)?;
            r.depth = r.depth.saturating_sub(1);
            if r.spent() {
                slot.take()
            } else {
                None
            }
        })
        .ok()
        .flatten();
    drop(spent);
}

#[cfg(not(test))]
fn button_down() -> bool {
    crate::input::primary_button_down()
}

/// 넘긴 `SC_MOVE` 의 OS 루프 — `DefSubclassProc` 안에서 돈다.
#[cfg(not(test))]
unsafe fn run_os_loop(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    DefSubclassProc(hwnd, msg, wp, lp)
}

// ADR-0295
unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _subclass_id: usize,
    refdata: usize,
) -> LRESULT {
    let id = refdata as u64;
    if !enter_frame(id) {
        return DefSubclassProc(hwnd, msg, wp, lp);
    }
    let result = match msg {
        m if Some(m) == begin_msg() && wp.0 == id as usize => on_begin(id, hwnd, lp),
        // 전제: 이 서브클래스가 이 창에 마지막으로 단 것이다(끌기 시작 때 단다 — tao · wry 는 창을 지을 때) — 표가 선
        //   동안 위에서 끼어들어 먼저 보낸 `WM_SYSCOMMAND` 가 없다.
        WM_SYSCOMMAND if with_loop(id, |r| std::mem::take(&mut r.in_flight)) == Some(true) => {
            run_passed_loop(id, hwnd, msg, wp, lp)
        }
        WM_ENTERSIZEMOVE => {
            // 아래 프로시저에 먼저 넘긴다 — 들어감 알림을 받은 쪽이 그 자리에서 창을 부수면 그 뒤에 넘길 창이 없다.
            //   들어감 처리(`Entering`)는 이 처리기가 돌아갈 때까지다.
            let r = DefSubclassProc(hwnd, msg, wp, lp);
            on_enter(id, hwnd);
            with_loop(id, |r| r.core.entry_done());
            r
        }
        WM_NCDESTROY => {
            if let Some(end) = with_loop(id, |r| r.core.on_destroyed()).flatten() {
                finish(id, end);
            }
            DefSubclassProc(hwnd, msg, wp, lp)
        }
        _ => DefSubclassProc(hwnd, msg, wp, lp),
    };
    leave_frame(id);
    result
}

/// 시작 신호가 닿았다 — Posted 확인 뒤 넘길 때만 진짜 `SC_MOVE` 를 보낸다. 신호는 이 루프의 것이라 아래로 넘기지
/// 않는다.
unsafe fn on_begin(id: u64, hwnd: HWND, lp: LPARAM) -> LRESULT {
    let button = button_down();
    match with_loop(id, |r| r.core.on_arrival(button)) {
        Some(Arrival::Swallow(end)) => {
            tracing::debug!(
                hwnd = hwnd_value(hwnd),
                loop_id = id,
                ?end,
                "들어가기 전에 끝냈다 — SC_MOVE 를 보내지 않는다"
            );
            finish(id, end);
        }
        Some(Arrival::Pass) => {
            with_loop(id, |r| r.in_flight = true);
            SendMessageW(hwnd, WM_SYSCOMMAND, WPARAM(SC_DRAGMOVE), lp);
            // 보낸 `SC_MOVE` 가 이 서브클래스에 닿지 않았으면(위에 단 다른 서브클래스가 먹었다) 루프 없이 돌아왔다 —
            // 여기서 끝을 낸다. 닿았으면 그 틀이 이미 끝을 냈고 `Core` 가 `None` 을 돌려준다.
            if let Some(end) = with_loop(id, |r| {
                r.in_flight = false;
                r.core.on_returned()
            })
            .flatten()
            {
                finish(id, end);
            }
        }
        Some(Arrival::NotOurs) | None => {}
    }
    LRESULT(0)
}

/// 이 루프가 보낸 `SC_MOVE` — OS 루프는 그 안에서 돈다.
unsafe fn run_passed_loop(id: u64, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let result = run_os_loop(hwnd, msg, wp, lp);
    if let Some(end) = with_loop(id, |r| r.core.on_returned()).flatten() {
        finish(id, end);
    }
    result
}

fn on_enter(id: u64, hwnd: HWND) {
    let button = button_down();
    let Some(post) = with_loop(id, |r| r.core.on_enter(button)).flatten() else {
        return;
    };
    tracing::debug!(
        hwnd = hwnd_value(hwnd),
        loop_id = id,
        button,
        "창 이동 루프에 들어갔다"
    );
    if post {
        post_cancel(hwnd);
    }
    with_loop(id, |r| r.outbox.push_back(LoopEvent::Entered));
    deliver(id);
}

unsafe extern "system" fn key_hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && wp.0 == VK_ESCAPE.0 as usize {
        on_escape();
    }
    CallNextHookEx(HHOOK::default(), code, wp, lp)
}

unsafe extern "system" fn mouse_hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && wp.0 as u32 == WM_LBUTTONUP {
        with_current(|r| r.core.on_mouse_up());
    }
    CallNextHookEx(HHOOK::default(), code, wp, lp)
}

/// Esc — 전환 끄기는 늘 · 숨기기는 깃발일 때만(hwnd_message_handler.cc#189-201 · TRD E9). 훅은 먹지 않는다.
fn on_escape() {
    let Some((id, hwnd, hide)) =
        with_current(|r| r.core.on_escape().map(|hide| (r.id, r.hwnd, hide))).flatten()
    else {
        return;
    };
    if set_transitions_disabled(hwnd, true) {
        with_loop(id, |r| r.dwm_disabled = true);
    }
    if hide {
        // 돌려주는 값은 전의 보임이다.
        // SAFETY: 이 루프의 창 하나.
        let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
    }
}

#[cfg(test)]
use tests::{button_down, run_os_loop};

#[cfg(test)]
mod tests {
    //! 실 Win32 창 위의 시험 — 루프 자체(OS 모달 루프 · 실 마우스)는 돌리지 않는다(TRD §12). 루프를 도는 자리
    //! (`run_os_loop`)와 버튼 읽기(`button_down`)를 스레드 지역 seam 으로 바꿔 끼운다.

    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetCapture, SetCapture};
    use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows::Win32::UI::WindowsAndMessaging::{
        DestroyWindow, DispatchMessageW, GetMessagePos, IsWindowVisible, PeekMessageW,
        SendMessageW, TranslateMessage, MSG, PM_REMOVE, WM_CANCELMODE, WM_CAPTURECHANGED,
        WM_ENTERSIZEMOVE, WM_NCDESTROY, WM_SYSCOMMAND,
    };

    use super::super::{
        begin_move_loop, end_move_loop, LoopEvent, MoveLoopEnd, MoveLoopError, MoveLoopOpts,
    };
    use super::{begin_msg, key_hook_seen_escape, mouse_hook_seen_release, SC_DRAGMOVE, SLOT};
    use crate::test_window::TestWindow;

    const RECORDER_ID: usize = 0x7E57;
    const BLOCKER_ID: usize = 0xB10C;
    const DESTROYER_ID: usize = 0xDE57;

    thread_local! {
        static BUTTON: Cell<bool> = const { Cell::new(false) };
        static STAND_IN: RefCell<Option<Box<dyn FnOnce(HWND)>>> = const { RefCell::new(None) };
        static SEEN: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
    }

    pub(super) fn button_down() -> bool {
        BUTTON.with(Cell::get)
    }

    /// 넘긴 `SC_MOVE` 자리 — 시험이 끼운 것을 한 번 돌린다(OS 모달 루프를 세우지 않는다).
    pub(super) unsafe fn run_os_loop(hwnd: HWND, _msg: u32, _wp: WPARAM, _lp: LPARAM) -> LRESULT {
        if let Some(f) = STAND_IN.with(|s| s.borrow_mut().take()) {
            f(hwnd);
        }
        LRESULT(0)
    }

    /// 우리 서브클래스 아래(먼저 단 서브클래스는 나중에 불린다)에서 넘어온 메시지를 적는다. `WM_SYSCOMMAND` 는 OS 에
    /// 넘기지 않는다 — 넘기면 시험 스레드에 진짜 모달 이동 루프가 선다.
    unsafe extern "system" fn recorder(
        hwnd: HWND,
        msg: u32,
        wp: WPARAM,
        lp: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        if matches!(msg, WM_SYSCOMMAND | WM_ENTERSIZEMOVE | WM_CANCELMODE) {
            SEEN.with(|s| s.borrow_mut().push(msg));
        }
        if msg == WM_SYSCOMMAND {
            return LRESULT(0);
        }
        if msg == WM_NCDESTROY {
            let _ = RemoveWindowSubclass(hwnd, Some(recorder), RECORDER_ID);
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }

    /// 우리 위에 단 서브클래스가 `WM_SYSCOMMAND` 를 먹는다 — 보낸 `SC_MOVE` 가 우리에게 닿지 않는 갈래.
    unsafe extern "system" fn blocker(
        hwnd: HWND,
        msg: u32,
        wp: WPARAM,
        lp: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        if msg == WM_SYSCOMMAND {
            return LRESULT(0);
        }
        if msg == WM_NCDESTROY {
            let _ = RemoveWindowSubclass(hwnd, Some(blocker), BLOCKER_ID);
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }

    /// 캡처를 잃으면 그 창을 부순다 — [`begin`](super::begin) 의 캡처 풀기 안 재진입 소멸 갈래.
    unsafe extern "system" fn destroyer(
        hwnd: HWND,
        msg: u32,
        wp: WPARAM,
        lp: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        if msg == WM_CAPTURECHANGED {
            let _ = DestroyWindow(hwnd);
            return LRESULT(0);
        }
        if msg == WM_NCDESTROY {
            let _ = RemoveWindowSubclass(hwnd, Some(destroyer), DESTROYER_ID);
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }

    struct Fixture {
        window: TestWindow,
        events: Rc<RefCell<Vec<LoopEvent>>>,
    }

    impl Fixture {
        fn new(button: bool) -> Self {
            BUTTON.with(|b| b.set(button));
            SEEN.with(|s| s.borrow_mut().clear());
            let window = TestWindow::new(true);
            // SAFETY: 이 시험 스레드의 살아 있는 창.
            assert!(
                unsafe { SetWindowSubclass(window.hwnd(), Some(recorder), RECORDER_ID, 0) }
                    .as_bool()
            );
            Self {
                window,
                events: Rc::new(RefCell::new(Vec::new())),
            }
        }

        fn begin(&self, hide_on_escape: bool) {
            let sink = Rc::clone(&self.events);
            begin_move_loop(
                &self.window,
                MoveLoopOpts { hide_on_escape },
                Box::new(move |ev| sink.borrow_mut().push(ev)),
            )
            .expect("루프 올리기");
        }

        fn events(&self) -> Vec<LoopEvent> {
            self.events.borrow().clone()
        }
    }

    fn pump() {
        let mut msg = MSG::default();
        // SAFETY: 이 스레드의 큐를 비운다 — 지역 MSG 하나.
        unsafe {
            while PeekMessageW(&mut msg, HWND::default(), 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    fn seen() -> Vec<u32> {
        SEEN.with(|s| s.borrow().clone())
    }

    fn slot_empty() -> bool {
        SLOT.with(|s| s.borrow().is_none())
    }

    fn ended(end: MoveLoopEnd, hid: bool) -> LoopEvent {
        LoopEvent::Ended { end, hid }
    }

    fn stand_in(f: impl FnOnce(HWND) + 'static) {
        STAND_IN.with(|s| *s.borrow_mut() = Some(Box::new(f)));
    }

    fn enter(hwnd: HWND) {
        // SAFETY: 이 시험 스레드의 창에 값 인자 메시지 하나.
        unsafe { SendMessageW(hwnd, WM_ENTERSIZEMOVE, WPARAM(0), LPARAM(0)) };
    }

    // ── Posted — 시작 신호가 닿을 때 끝냄(SC_MOVE 를 아예 안 보낸다) ──

    #[test]
    fn end_before_arrival_never_sends_sc_move_and_ends_once() {
        let f = Fixture::new(true);
        f.begin(false);
        end_move_loop(&f.window);
        assert!(f.events().is_empty(), "끝은 시작 신호가 닿을 때 낸다");
        pump();
        assert_eq!(f.events(), vec![ended(MoveLoopEnd::Ended, false)]);
        assert!(!seen().contains(&WM_SYSCOMMAND), "SC_MOVE 를 보내지 않았다");
        assert!(slot_empty());
        end_move_loop(&f.window);
        pump();
        assert_eq!(f.events().len(), 1, "끝난 뒤 end_move_loop 는 무동작");
    }

    #[test]
    fn released_before_arrival_ends_released_before_entry() {
        let f = Fixture::new(false);
        f.begin(false);
        pump();
        assert_eq!(
            f.events(),
            vec![ended(MoveLoopEnd::Released { before_entry: true }, false)]
        );
        assert!(!seen().contains(&WM_SYSCOMMAND));
    }

    #[test]
    fn foreign_sc_move_with_identical_coordinates_is_not_ours() {
        let f = Fixture::new(false);
        f.begin(false);
        // 같은 창에 따로 온 `SC_MOVE | 2` — 값 · 자리가 올릴 때와 똑같다(웹뷰의 drag 영역이 보내는 것 따위).
        // SAFETY: 인자 없는 조회와, 이 시험 스레드의 창에 값 인자 메시지 하나(기록기가 OS 에 넘기지 않는다).
        unsafe {
            let pos = GetMessagePos();
            SendMessageW(
                f.window.hwnd(),
                WM_SYSCOMMAND,
                WPARAM(SC_DRAGMOVE),
                LPARAM(pos as isize),
            )
        };
        assert_eq!(seen(), vec![WM_SYSCOMMAND], "남의 것은 아래로 넘긴다");
        assert!(f.events().is_empty());
        assert!(
            SLOT.with(|s| s.borrow().as_ref().is_some_and(|r| !r.core.is_done())),
            "이 루프는 그대로 Posted"
        );
        pump();
        assert_eq!(
            f.events(),
            vec![ended(MoveLoopEnd::Released { before_entry: true }, false)]
        );
        assert_eq!(seen(), vec![WM_SYSCOMMAND]);
    }

    #[test]
    fn begin_signal_with_another_loop_number_is_not_ours() {
        let f = Fixture::new(true);
        f.begin(false);
        let id = SLOT.with(|s| s.borrow().as_ref().unwrap().id);
        // SAFETY: 이 시험 스레드의 창에 값 인자 메시지 하나.
        unsafe {
            SendMessageW(
                f.window.hwnd(),
                begin_msg().unwrap(),
                WPARAM(id as usize + 1000),
                LPARAM(0),
            )
        };
        assert!(f.events().is_empty());
        assert!(SLOT.with(|s| s.borrow().as_ref().is_some_and(|r| !r.core.is_done())));
        end_move_loop(&f.window);
        pump();
        assert_eq!(f.events(), vec![ended(MoveLoopEnd::Ended, false)]);
    }

    #[test]
    fn escape_before_arrival_hides_and_ends_canceled() {
        let f = Fixture::new(true);
        crate::window::place_window(
            &f.window,
            Some(crate::window::PhysRect {
                x: -24000,
                y: -24000,
                width: 100,
                height: 80,
            }),
            crate::window::ZOrder::Keep,
            true,
            false,
        )
        .unwrap();
        f.begin(true);
        key_hook_seen_escape();
        // SAFETY: 이 시험의 살아 있는 창.
        assert!(
            !unsafe { IsWindowVisible(f.window.hwnd()) }.as_bool(),
            "hide_on_escape 면 숨긴다"
        );
        pump();
        assert_eq!(
            f.events(),
            vec![ended(MoveLoopEnd::Canceled { esc: true }, true)]
        );
        assert!(!seen().contains(&WM_SYSCOMMAND));
    }

    #[test]
    fn end_and_escape_before_arrival_end_as_ended_with_hid() {
        let f = Fixture::new(true);
        f.begin(true);
        end_move_loop(&f.window);
        key_hook_seen_escape();
        pump();
        assert_eq!(f.events(), vec![ended(MoveLoopEnd::Ended, true)]);
    }

    #[test]
    fn escape_without_hide_flag_never_hides() {
        let f = Fixture::new(true);
        crate::window::place_window(&f.window, None, crate::window::ZOrder::Keep, true, false)
            .unwrap();
        f.begin(false);
        key_hook_seen_escape();
        // SAFETY: 이 시험의 살아 있는 창.
        assert!(unsafe { IsWindowVisible(f.window.hwnd()) }.as_bool());
        pump();
        assert_eq!(
            f.events(),
            vec![ended(MoveLoopEnd::Canceled { esc: true }, false)]
        );
    }

    #[test]
    fn destroy_before_arrival_ends_destroyed_once() {
        let f = Fixture::new(true);
        f.begin(false);
        // SAFETY: 이 시험의 창.
        unsafe { DestroyWindow(f.window.hwnd()) }.unwrap();
        pump();
        assert_eq!(f.events(), vec![ended(MoveLoopEnd::Destroyed, false)]);
        assert!(slot_empty());
    }

    #[test]
    fn destroy_while_releasing_capture_is_ok_with_one_end() {
        let f = Fixture::new(true);
        let hwnd = f.window.hwnd();
        // SAFETY: 이 시험 스레드의 살아 있는 창 — 캡처를 쥐고, 잃을 때 창을 부수는 서브클래스를 단다.
        unsafe {
            assert!(SetWindowSubclass(hwnd, Some(destroyer), DESTROYER_ID, 0).as_bool());
            let _ = SetCapture(hwnd);
            assert_eq!(GetCapture(), hwnd, "시험 전제 — 이 창이 캡처를 쥐었다");
        }
        f.begin(false);
        assert_eq!(f.events(), vec![ended(MoveLoopEnd::Destroyed, false)]);
        assert!(slot_empty());
        pump();
        assert_eq!(f.events().len(), 1);
    }

    // ── 넘긴 뒤(루프 자리 = 시험이 끼운 것) ──

    #[test]
    fn passed_loop_enters_then_releases() {
        let f = Fixture::new(true);
        f.begin(false);
        let events = Rc::clone(&f.events);
        stand_in(move |hwnd| {
            enter(hwnd);
            assert_eq!(*events.borrow(), vec![LoopEvent::Entered]);
            mouse_hook_seen_release();
        });
        pump();
        assert_eq!(
            f.events(),
            vec![
                LoopEvent::Entered,
                ended(
                    MoveLoopEnd::Released {
                        before_entry: false
                    },
                    false
                )
            ]
        );
        assert!(seen().contains(&WM_ENTERSIZEMOVE));
        assert!(slot_empty());
    }

    #[test]
    fn sc_move_eaten_above_us_still_ends_once() {
        let f = Fixture::new(true);
        f.begin(false);
        // SAFETY: 이 시험 스레드의 창 — 우리 뒤에 달아 우리 위에서 불린다.
        assert!(
            unsafe { SetWindowSubclass(f.window.hwnd(), Some(blocker), BLOCKER_ID, 0) }.as_bool()
        );
        let ran = Rc::new(Cell::new(false));
        let flag = Rc::clone(&ran);
        stand_in(move |_| flag.set(true));
        pump();
        assert!(!ran.get(), "보낸 SC_MOVE 가 우리에게 닿지 않았다");
        assert_eq!(
            f.events(),
            vec![ended(MoveLoopEnd::Canceled { esc: false }, false)]
        );
        assert!(slot_empty());
    }

    #[test]
    fn button_up_at_entry_posts_cancel_instead_of_sending() {
        let f = Fixture::new(true);
        f.begin(false);
        stand_in(|hwnd| {
            BUTTON.with(|b| b.set(false));
            enter(hwnd);
            assert!(
                !seen().contains(&WM_CANCELMODE),
                "들어가는 처리기 안에서는 보내지 않는다"
            );
        });
        pump();
        assert!(seen().contains(&WM_CANCELMODE), "올린 취소는 펌프가 나른다");
        assert_eq!(
            f.events(),
            vec![
                LoopEvent::Entered,
                ended(MoveLoopEnd::Released { before_entry: true }, false)
            ]
        );
    }

    #[test]
    fn end_while_running_sends_cancel_synchronously() {
        let f = Fixture::new(true);
        f.begin(false);
        let window = f.window.handle_only();
        stand_in(move |hwnd| {
            enter(hwnd);
            end_move_loop(&window);
            assert!(seen().contains(&WM_CANCELMODE), "돌고 있으면 곧바로 보낸다");
        });
        pump();
        assert_eq!(
            f.events(),
            vec![LoopEvent::Entered, ended(MoveLoopEnd::Ended, false)]
        );
    }

    #[test]
    fn end_from_another_thread_does_nothing() {
        let f = Fixture::new(true);
        f.begin(false);
        let raw = f.window.handle_only();
        std::thread::spawn(move || end_move_loop(&raw))
            .join()
            .unwrap();
        stand_in(enter);
        pump();
        assert_eq!(
            f.events(),
            vec![
                LoopEvent::Entered,
                ended(MoveLoopEnd::Canceled { esc: false }, false)
            ],
            "다른 스레드의 끝냄 청은 적히지 않았다"
        );
    }

    #[test]
    fn escape_seen_then_release_keeps_hid() {
        let f = Fixture::new(true);
        f.begin(true);
        stand_in(|hwnd| {
            enter(hwnd);
            key_hook_seen_escape();
            mouse_hook_seen_release();
        });
        pump();
        assert_eq!(
            f.events(),
            vec![
                LoopEvent::Entered,
                ended(
                    MoveLoopEnd::Released {
                        before_entry: false
                    },
                    true
                )
            ]
        );
    }

    #[test]
    fn loop_end_without_release_is_cancel() {
        let f = Fixture::new(true);
        f.begin(false);
        stand_in(enter);
        pump();
        assert_eq!(
            f.events(),
            vec![
                LoopEvent::Entered,
                ended(MoveLoopEnd::Canceled { esc: false }, false)
            ]
        );
    }

    #[test]
    fn destroy_inside_the_loop_ends_once_and_frees_at_depth_zero() {
        // E10 — 루프를 도는 틀 안에서 창이 부서진다.
        let f = Fixture::new(true);
        f.begin(false);
        let events = Rc::clone(&f.events);
        stand_in(move |hwnd| {
            enter(hwnd);
            // SAFETY: 이 시험의 창.
            unsafe { DestroyWindow(hwnd) }.unwrap();
            assert_eq!(
                events.borrow().last(),
                Some(&ended(MoveLoopEnd::Destroyed, false))
            );
            assert!(
                !slot_empty(),
                "WM_SYSCOMMAND 틀이 아직 걸려 있어 칸을 비우지 않는다"
            );
        });
        pump();
        assert_eq!(
            f.events(),
            vec![LoopEvent::Entered, ended(MoveLoopEnd::Destroyed, false)]
        );
        assert!(slot_empty(), "깊이 0 으로 돌아온 틀이 비운다");
    }

    #[test]
    fn destroy_inside_the_entered_callback_still_ends_once_after_entered() {
        let f = Fixture::new(true);
        let events = Rc::clone(&f.events);
        let hwnd = f.window.hwnd();
        begin_move_loop(
            &f.window,
            MoveLoopOpts::default(),
            Box::new(move |ev| {
                events.borrow_mut().push(ev);
                if ev == LoopEvent::Entered {
                    // SAFETY: 이 시험의 창 — 받는 쪽이 들어감 알림 안에서 창을 부순다.
                    unsafe { DestroyWindow(hwnd) }.unwrap();
                }
            }),
        )
        .unwrap();
        stand_in(enter);
        pump();
        assert_eq!(
            f.events(),
            vec![LoopEvent::Entered, ended(MoveLoopEnd::Destroyed, false)]
        );
        assert!(slot_empty());
    }

    // ── 한 스레드에 하나 ──

    #[test]
    fn second_loop_is_busy_until_the_first_ends_then_can_start_from_its_end() {
        let first = Fixture::new(false);
        let second = TestWindow::new(true);
        first.begin(false);
        let err = begin_move_loop(&second, MoveLoopOpts::default(), Box::new(|_| {})).unwrap_err();
        assert!(matches!(err, MoveLoopError::Busy));

        // 끝 알림 안에서 다음 루프를 올릴 수 있다.
        let started = Rc::new(Cell::new(false));
        let flag = Rc::clone(&started);
        let next = second.handle_only();
        SLOT.with(|s| {
            s.borrow_mut().as_mut().unwrap().callback = Some(Box::new(move |ev| {
                if matches!(ev, LoopEvent::Ended { .. }) {
                    flag.set(
                        begin_move_loop(&next, MoveLoopOpts::default(), Box::new(|_| {})).is_ok(),
                    );
                }
            }));
        });
        pump();
        assert!(started.get());
        end_move_loop(&second);
        pump();
        assert!(slot_empty());
    }

    #[test]
    fn other_thread_is_wrong_thread() {
        let window = TestWindow::new(true);
        let raw = window.handle_only();
        let err = std::thread::spawn(move || {
            begin_move_loop(&raw, MoveLoopOpts::default(), Box::new(|_| {})).unwrap_err()
        })
        .join()
        .unwrap();
        assert!(matches!(err, MoveLoopError::WrongThread));
    }

    #[test]
    fn decode_pos_sign_extends_both_halves() {
        assert_eq!(super::decode_pos(0x0020_0010), (16, 32));
        assert_eq!(super::decode_pos(0xFFFF_FFF6), (-10, -1));
    }
}

#[cfg(test)]
fn key_hook_seen_escape() {
    on_escape();
}

#[cfg(test)]
fn mouse_hook_seen_release() {
    with_current(|r| r.core.on_mouse_up());
}
