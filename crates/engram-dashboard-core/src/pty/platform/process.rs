//! PID liveness + 프로세스 시작시각(creation time) 헬퍼.
//!
//! ★왜 core 에 두나★: daemon(portfile)·tauri(discovery) 양쪽이 "데몬 PID 가 살아있는가"를
//! 판정해야 한다. 두 crate 모두 core 에 의존하고 core 는 이미 windows 에 의존하므로,
//! 판정 로직을 여기 한 곳에 두고 양쪽이 재사용한다(DRY — 사본 중복/무테스트 제거).
//!
//! ★왜 creation time 까지 보나★: PID 는 OS 가 재사용한다. 데몬이 죽고 같은 PID 를 다른
//! 프로세스가 받으면 "PID 살아있음"만으로는 false-live(엉뚱한 프로세스를 데몬으로 오인)가
//! 난다. 그래서 "PID 살아있음 AND 그 PID 의 현재 creation time == 기록된 값"으로 판정해
//! PID 재사용을 직접 구분한다. creation time 은 GetProcessTimes 의 lpCreationTime(FILETIME)을
//! u64(100ns 단위, 1601-01-01 기준)로 합친 값이다 — 같은 프로세스면 불변, 재사용 PID 면 다르다.

/// 주어진 PID 의 프로세스 시작시각(creation FILETIME)을 u64 로 반환. 조회 실패면 None.
///
/// u64 단위/의미: Windows FILETIME = 1601-01-01 UTC 부터 100나노초 간격 수.
/// high/low 32비트를 합쳐 u64 로 만든다. 조회 실패(부재/권한)는 None. PID 0 은 항상 None.
#[cfg(windows)]
pub fn process_creation_time(pid: u32) -> Option<u64> {
    use windows::Win32::Foundation::{CloseHandle, FILETIME};
    use windows::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    if pid == 0 {
        return None;
    }
    // SAFETY: 최소 권한으로 PID 핸들 open. 대상 부재/권한부족이면 Err.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;

    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: 방금 연 유효 핸들 + 스택의 4개 FILETIME 출력 포인터.
    let ok = unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) }
        .is_ok();
    // SAFETY: 유효 핸들 한 번 close.
    unsafe {
        let _ = CloseHandle(handle);
    }
    if !ok {
        return None;
    }
    Some(((creation.dwHighDateTime as u64) << 32) | (creation.dwLowDateTime as u64))
}

/// 현재(자기) 프로세스의 시작시각. 데몬이 daemon.json 에 기록할 값.
#[cfg(windows)]
pub fn current_process_start_time() -> Option<u64> {
    process_creation_time(std::process::id())
}

/// PID 단독 생존 판정(creation time 무시) — OpenProcess + GetExitCodeProcess.
/// start_time 미상(0)일 때의 보수 fallback 으로만 쓴다.
#[cfg(windows)]
pub fn pid_alive(pid: u32) -> bool {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    if pid == 0 {
        return false;
    }
    // SAFETY: 최소 권한으로 PID 핸들 open. 못 열면 부재/권한부족 — 보수적으로 살아있다고 본다.
    let handle = match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
        Ok(h) => h,
        Err(_) => return true,
    };
    const STILL_ACTIVE: u32 = 259;
    let mut code: u32 = 0;
    // SAFETY: 방금 연 유효 핸들 + 스택 출력 포인터.
    let ok = unsafe { GetExitCodeProcess(handle, &mut code) }.is_ok();
    // SAFETY: 유효 핸들 한 번 close.
    unsafe {
        let _ = CloseHandle(handle);
    }
    if !ok {
        return true; // 조회 실패 → 보수적으로 살아있음.
    }
    code == STILL_ACTIVE
}

/// PID 생존 판정(시작시각 대조 포함). true=살아있음(우리가 찾는 그 프로세스).
///
/// 판정 규칙(M2 PID 재사용 방어):
///   - pid==0 → 죽음(false). 시스템 idle 은 우리 데몬일 수 없다.
///   - expected_start==0(미상, 옛 daemon.json 호환) → creation time 대조 불가하므로
///     **PID 생존만으로 보수 판정**(살아있으면 live). 옛 파일을 함부로 stale 로 몰지 않는다.
///   - expected_start!=0 → PID 의 현재 creation time 이 expected_start 와 정확히 일치할 때만 live.
///     불일치(재사용 PID) 또는 조회 실패(프로세스 부재)면 dead.
#[cfg(windows)]
pub fn pid_alive_with_start_time(pid: u32, expected_start: u64) -> bool {
    if pid == 0 {
        return false;
    }
    match process_creation_time(pid) {
        Some(actual) => {
            if expected_start == 0 {
                // 미상 → PID 가 살아있다는 사실만으로 보수적으로 live.
                true
            } else {
                actual == expected_start
            }
        }
        // creation time 조회 실패 = 프로세스 부재(또는 권한). expected_start 가 미상이면
        // 보수적으로 OpenProcess 생존 fallback 으로 한 번 더 본다. 알려진 start_time 이 있는데
        // 조회조차 안 되면 데몬은 죽은 것으로 본다.
        None => {
            if expected_start == 0 {
                pid_alive(pid)
            } else {
                false
            }
        }
    }
}

// ── non-windows stub ─────────────────────────────────────────────────────────────

#[cfg(not(windows))]
pub fn process_creation_time(_pid: u32) -> Option<u64> {
    None
}

#[cfg(not(windows))]
pub fn current_process_start_time() -> Option<u64> {
    None
}

/// non-windows: 생존 판정 수단 미구현 — 보수적으로 살아있다고 본다(start_time 무시).
#[cfg(not(windows))]
pub fn pid_alive_with_start_time(pid: u32, _expected_start: u64) -> bool {
    pid != 0
}

#[cfg(not(windows))]
pub fn pid_alive(pid: u32) -> bool {
    pid != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_zero_is_not_alive() {
        // PID 0 = 시스템 idle. 어떤 경로든 live 가 아니어야 한다.
        assert!(!pid_alive(0));
        assert!(!pid_alive_with_start_time(0, 0));
        assert!(!pid_alive_with_start_time(0, 12345));
        assert_eq!(process_creation_time(0), None);
    }

    #[cfg(windows)]
    #[test]
    fn current_process_is_alive_with_matching_start_time() {
        // 자기 PID + 자기 creation time → live.
        let pid = std::process::id();
        let start = current_process_start_time().expect("자기 creation time 조회 가능");
        assert!(start != 0, "creation time 은 0 이 아니어야 함");
        assert!(pid_alive_with_start_time(pid, start), "자기 자신은 live");
        assert!(pid_alive(pid), "자기 자신은 OpenProcess 로도 live");
    }

    #[cfg(windows)]
    #[test]
    fn current_pid_with_wrong_start_time_is_dead() {
        // 같은 PID 라도 creation time 이 다르면(=재사용된 PID 시나리오) dead 로 판정.
        let pid = std::process::id();
        let real = current_process_start_time().unwrap();
        let wrong = real.wrapping_add(1); // 의도적으로 불일치
        assert!(
            !pid_alive_with_start_time(pid, wrong),
            "creation time 불일치면 dead(PID 재사용 방어)"
        );
    }

    #[cfg(windows)]
    #[test]
    fn unknown_start_time_falls_back_to_pid_liveness() {
        // start_time==0(미상, 옛 daemon.json) → PID 생존만으로 보수 판정(자기 PID 는 live).
        let pid = std::process::id();
        assert!(
            pid_alive_with_start_time(pid, 0),
            "미상이면 PID 생존으로 보수 판정"
        );
    }
}
