//! 데스크톱 창의 OS 규칙 — 지금 전경 창이 이 프로세스의 것인가.
//!
//! 셸이 런타임 복원 수락 끝에 main 창에 포커스를 줄지 가르는 데 쓴다(셸 `state/placement.rs` 의
//! `foreground_is_ours` · TRD S21-storage §6-7 ④). 저장 관리 브랜치가 셸 안에 `#[cfg(windows)]` 로 두었던 것을
//! master 머지 때 이 crate 로 옮겼다(ADR-0266 — 운영 OS 분기는 이 crate 안에만).

// ADR-0266

/// 지금 전경 창이 이 프로세스의 것인가. `None` = 이 OS 에서는 답할 수단이 이 crate 에 없다 — 부르는 쪽이 자기
/// 수단(창 툴킷의 포커스 게터 따위)으로 판정한다(lib.rs 「들이는 규칙」 3 — Windows 밖 갈래는 자리채움).
///
/// Windows = `GetForegroundWindow` 의 창을 만든 스레드의 PID 가 이 프로세스의 PID 와 같은가. 전경 창이 없거나 그
/// 사이 사라졌으면 `Some(false)` 다.
#[cfg(windows)]
pub fn foreground_is_current_process() -> Option<bool> {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    let mut pid = 0u32;
    // SAFETY: 인자 없는 조회 하나와, 살아 있는 지역 변수에 PID 를 쓰는 조회 하나다. 전경 창이 없거나 그 사이
    //   사라졌으면 PID 가 0 으로 남는다.
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
