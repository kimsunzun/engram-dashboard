//! 자식 프로세스를 띄울 때의 OS 설정.
// ADR-0266

use std::process::Command;

/// 콘솔 창 없이 띄운다 — Windows = `CREATE_NO_WINDOW`, 그 밖의 OS = 무동작. 창 없는 프로세스(데몬)가 콘솔 앱을
/// 그냥 띄우면 Windows 가 새 콘솔 창을 연다.
///
/// ★생성 플래그를 통째로 정한다★ — std 의 `creation_flags` 는 더하지 않고 바꾼다. 이 앞에 준 생성 플래그는 이것이
/// 지우고, 이 뒤에 생성 플래그를 다시 주면 이것이 지워진다.
pub fn hide_console_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows::Win32::System::Threading::CREATE_NO_WINDOW;
        cmd.creation_flags(CREATE_NO_WINDOW.0);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}
