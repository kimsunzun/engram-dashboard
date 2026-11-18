//! The current state of input devices — cursor position, primary button, whether Esc is pressed. Callable from any thread (the drop
//!   command reads it after the drag ends — TRD S21-slot-docking 7th edition §3-2).
//! All three read the OS's asynchronous state, bypassing the message queue, so webview capture or another process's window does not
//!   matter. ★If a secure desktop (lock screen, UAC) is in front the OS does not answer★ — the cursor is `None` and the
//! button reads as released (MS `GetCursorPos`, `GetAsyncKeyState` docs; TRD S21-slot-docking 6.3 §12 M14).

// ADR-0266, ADR-0295

use crate::window::PhysPoint;

/// The cursor position on screen (Win32 `GetCursorPos`). `None` = the OS did not answer (secure desktop etc.) or outside Windows.
pub fn cursor_pos() -> Option<PhysPoint> {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
        let mut p = POINT::default();
        // SAFETY: one query that writes into a live local point.
        unsafe { GetCursorPos(&mut p) }
            .ok()
            .map(|()| PhysPoint { x: p.x, y: p.y })
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Is the primary button pressed now. With the left-handed setting (`SM_SWAPBUTTON`) it is the physical right button — `GetAsyncKeyState`
///   looks at the physical button (MS docs). Outside Windows = false.
pub fn primary_button_down() -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
        use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_SWAPBUTTON};
        // SAFETY: two queries that only read values.
        unsafe {
            let swapped = GetSystemMetrics(SM_SWAPBUTTON) != 0;
            (GetAsyncKeyState(primary_vk(swapped).0 as i32) as u16 & 0x8000) != 0
        }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Is Esc pressed now (Win32 `GetAsyncKeyState(VK_ESCAPE)`). A very short Esc tap may already read as released (spike 2026-10-20: missed
///   1/5). Outside Windows = false.
// TRD S21-slot-docking §3-2
pub fn escape_down() -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_ESCAPE};
        // SAFETY: one query that only reads a value.
        unsafe { (GetAsyncKeyState(VK_ESCAPE.0 as i32) as u16 & 0x8000) != 0 }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(windows)]
fn primary_vk(swapped: bool) -> windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY {
    use windows::Win32::UI::Input::KeyboardAndMouse::{VK_LBUTTON, VK_RBUTTON};
    if swapped {
        VK_RBUTTON
    } else {
        VK_LBUTTON
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use windows::Win32::UI::Input::KeyboardAndMouse::{VK_LBUTTON, VK_RBUTTON};

    #[test]
    fn swapped_buttons_read_the_physical_right_button() {
        assert_eq!(primary_vk(false), VK_LBUTTON);
        assert_eq!(primary_vk(true), VK_RBUTTON);
    }

    /// It answers only when an interactive desktop is in front — on a CI runner or lock screen `None` may be correct, so run it by hand.
    #[test]
    #[ignore = "by hand on an interactive desktop"]
    fn cursor_and_button_answer_on_an_interactive_desktop() {
        assert!(cursor_pos().is_some());
        assert!(
            !primary_button_down(),
            "do not press the primary button during the test"
        );
    }
}

// It compiles regardless of OS — outside Windows it is always false, so the same assertion holds.
#[cfg(test)]
mod portable_tests {
    use super::*;

    // A CI runner or an untouched dev PC — nobody is holding Esc.
    #[test]
    fn escape_reads_up_when_nobody_holds_it() {
        assert!(!escape_down());
    }
}
