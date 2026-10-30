//! 실프로세스 시험 도우미 — 이 crate 의 시험과, `test-support` 기능을 dev 의존으로 켠 부르는 쪽 시험이 함께
//! 쓴다(TRD 1-3 §3-8 — `docs/process/S21-crate-boundaries/trd-1-3-platform-crate.md`). 모두 OS 수준 값만
//! 주고받는다(`Child` · PID · 시작시각 · 탐침).
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

/// 그 PID 가 지금도 `start_time`(프로세스 생성 FILETIME — [`crate::process::process_creation_time`] 과 같은 척도)에
/// 태어난 그 프로세스이고 아직 돌면 끝낸다. 시작시각 확인 · 생존 확인 · 끝내기를 **같은 손잡이**로 한다 — 손잡이를
/// 쥔 동안 그 PID 는 남에게 넘어가지 않으므로, 확인한 뒤 엉뚱한 프로세스를 끄는 길이 없다. 자손은 건드리지 않는다.
///
/// `Ok(true)` = 끝내기를 맡겼다(기다리지 않는다) · `Ok(false)` = 그 프로세스가 아니다(없다 · 시작시각이 다르다 · 이미
/// 끝났다) · `Err` = 열거나 읽거나 끝내지 못했다(권한 거부 · 확인과 끝내기 사이에 스스로 끝난 경우 포함) — 끄지 않았다.
#[cfg(windows)]
pub fn terminate_if_same(pid: u32, start_time: u64) -> std::io::Result<bool> {
    use windows::core::HRESULT;
    use windows::Win32::Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, FILETIME, HANDLE};
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, GetProcessTimes, OpenProcess, TerminateProcess,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
    };
    const STILL_ACTIVE: u32 = 259;

    fn decide(handle: HANDLE, start_time: u64) -> windows::core::Result<bool> {
        let mut created = FILETIME::default();
        let mut exited = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        // SAFETY: 부른 쪽이 연 유효 핸들 + 스택의 출력 넷.
        unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) }?;
        let created = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
        if created != start_time {
            return Ok(false);
        }
        let mut code = 0u32;
        // SAFETY: 같은 핸들 + 스택 출력.
        unsafe { GetExitCodeProcess(handle, &mut code) }?;
        if code != STILL_ACTIVE {
            return Ok(false);
        }
        // SAFETY: 같은 핸들 — 끝내기 권한으로 열었다.
        unsafe { TerminateProcess(handle, 1) }?;
        Ok(true)
    }

    let access = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE;
    // SAFETY: 조회 · 끝내기 권한만 연다.
    let handle = match unsafe { OpenProcess(access, false, pid) } {
        Ok(h) => h,
        Err(e) if e.code() == HRESULT::from_win32(ERROR_INVALID_PARAMETER.0) => return Ok(false),
        Err(e) => return Err(std::io::Error::other(format!("PID {pid} 열기: {e}"))),
    };
    let outcome = decide(handle, start_time)
        .map_err(|e| std::io::Error::other(format!("PID {pid} 확인 · 끝내기: {e}")));
    // SAFETY: 위에서 연 핸들을 한 번 닫는다.
    unsafe {
        let _ = CloseHandle(handle);
    }
    outcome
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::process::{pid_alive, process_creation_time};

    /// 시작시각이 다른 신원으로는 끄지 않고 같은 신원으로만 끈다 — 끝난 뒤에는 같은 신원도 「그 프로세스가 아니다」다.
    #[test]
    fn terminate_if_same_ends_only_the_matching_identity() {
        let mut child = spawn_gated_cmd("rem", 0);
        let pid = child.id();
        let start = process_creation_time(pid).expect("시작시각");

        let other = terminate_if_same(pid, start + 1);
        let alive_after_other = pid_alive(pid);
        let same = terminate_if_same(pid, start);
        let _ = child.kill();
        let _ = child.wait();
        let after_exit = terminate_if_same(pid, start);

        assert!(matches!(other, Ok(false)), "{other:?}");
        assert!(alive_after_other, "시작시각이 다른 신원으로 껐다");
        assert!(matches!(same, Ok(true)), "{same:?}");
        assert!(matches!(after_exit, Ok(false)), "{after_exit:?}");
    }
}
