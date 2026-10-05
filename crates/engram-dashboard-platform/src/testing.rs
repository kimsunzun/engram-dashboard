//! 실프로세스 시험 도우미 — 이 crate 의 시험과, `test-support` 기능을 dev 의존으로 켠 부르는 쪽 시험이 함께
//! 쓴다(TRD 1-3 §3-8 — `docs/process/S21-crate-boundaries/trd-1-3-platform-crate.md`). 넷 다 OS 수준 값만
//! 주고받는다(`Child` · PID · 탐침).
//!
//! 무리 손잡이를 만드는 도우미는 두지 않는다 — 부르는 쪽 시험이 쓰는 손잡이는 그쪽 어댑터 타입이라 여기서 못
//! 만든다. 무리 자체는 [`crate::group::GroupOwner`] 로 만든다.
// ADR-0275

use std::io::Write;
use std::process::Child;
use std::time::{Duration, Instant};

/// `cmd.exe /d /c <line>` — 단 stdin 에서 한 줄을 받을 때까지 `line` 을 시작하지 않는다. 그 사이에 무리에 넣으면
/// `line` 이 띄우는 것은 전부 무리 안에서 태어난다. [`open_gate`] 가 그 한 줄을 준다. `flags` = 생성 플래그 그대로.
/// 띄우기에 실패하면 패닉한다.
#[cfg(windows)]
pub fn spawn_gated_cmd(line: &str, flags: u32) -> Child {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    Command::new("cmd.exe")
        .raw_arg(format!("/d /c set /p _= & {line}"))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(flags)
        .spawn()
        .expect("cmd.exe 기동")
}

/// [`spawn_gated_cmd`] 의 문을 연다 — stdin 에 한 줄을 쓰고 그 끝을 닫는다. stdin 이 이미 빠졌으면 패닉한다.
pub fn open_gate(child: &mut Child) {
    let mut stdin = child.stdin.take().expect("stdin 파이프");
    let _ = stdin.write_all(b"go\r\n");
}

/// `probe` 가 `Some` 을 줄 때까지 20 ms 마다 묻는다 — 10 초 안에 못 보면 `what` 을 담아 패닉한다.
///
/// base 의 `testing::wait_until`(시한 · 조건 → `bool`)과 다른 함수다 — 이 crate 는 base 를 의존하지 않아 합치지
/// 않는다(ADR-0266 · ADR-0268).
pub fn wait_until<T>(what: &str, mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(found) = probe() {
            return found;
        }
        assert!(Instant::now() < deadline, "10 초 안에 {what} 를 못 봤다");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// 그 번호의 실행 파일이 `ping.exe` 인가. 못 열면 `false`.
#[cfg(windows)]
pub fn is_ping(pid: u32) -> bool {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    // SAFETY: 조회 권한만 연다.
    let Ok(handle) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else {
        return false;
    };
    let image = crate::group::image_path(handle);
    // SAFETY: 방금 연 핸들을 한 번 닫는다.
    unsafe {
        let _ = CloseHandle(handle);
    }
    image.is_some_and(|p| p.to_ascii_lowercase().ends_with("\\ping.exe"))
}
