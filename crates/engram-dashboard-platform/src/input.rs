//! 입력 장치의 지금 상태 — 커서 자리 · 주 버튼이 눌려 있나. 어느 스레드에서나 부른다(셸의 앱 수명 폴러 스레드 ·
//! 메인 스레드 · 이동 루프의 들어가기 전 · 들어갈 때 확인 — TRD S21-slot-docking §8).
//!
//! 둘 다 OS 의 비동기 상태를 읽는다 — 메시지 큐를 거치지 않으므로 웹뷰가 쥔 캡처 · 다른 프로세스의 창과 무관하게
//! 지금 장치 상태다. ★보안 데스크톱(잠금 화면 · UAC)이 앞에 있으면 OS 가 답하지 않는다★ — 커서는 `None`, 버튼은
//! 떨어진 것으로 온다(MS `GetCursorPos` · `GetAsyncKeyState` 문서 · TRD §12 M14).

// ADR-0266
// ADR-0295

use crate::window::PhysPoint;

/// 화면의 커서 자리(Win32 `GetCursorPos`). `None` = OS 가 답하지 않았다(보안 데스크톱 따위) · Windows 밖.
pub fn cursor_pos() -> Option<PhysPoint> {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
        let mut p = POINT::default();
        // SAFETY: 살아 있는 지역 점에 쓰는 조회 하나.
        unsafe { GetCursorPos(&mut p) }
            .ok()
            .map(|()| PhysPoint { x: p.x, y: p.y })
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// 주 버튼이 지금 눌려 있나. 왼손잡이 설정(단추 바꿈 `SM_SWAPBUTTON`)이면 물리 오른쪽 버튼이다 —
/// `GetAsyncKeyState` 는 논리 버튼이 아니라 물리 버튼을 본다(MS 문서). Windows 밖 = 거짓.
pub fn primary_button_down() -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
        use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_SWAPBUTTON};
        // SAFETY: 값만 읽는 조회 둘.
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

    /// 대화형 데스크톱이 앞에 있어야 답한다 — CI 러너 · 잠금 화면에서는 `None` 이 정답일 수 있어 손으로 돌린다.
    #[test]
    #[ignore = "대화형 데스크톱에서 손으로"]
    fn cursor_and_button_answer_on_an_interactive_desktop() {
        assert!(cursor_pos().is_some());
        assert!(
            !primary_button_down(),
            "시험 중에는 주 버튼을 누르지 않는다"
        );
    }
}
