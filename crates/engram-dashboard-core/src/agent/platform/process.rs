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

/// 주어진 부모 PID 의 **직계 자식 프로세스 PID 목록**을 OS 스냅샷으로 열거한다(Windows).
///
/// ★왜 필요한가★: AgentInfo/WS 프로토콜은 PTY child 의 PID 를 노출하지 않는다(설계상 손발/두뇌
/// 분리 — 프론트는 PID 를 몰라도 된다). 그러나 실프로세스 격리테스트(데몬 .exe kill → PTY child
/// 동반 사망)는 "데몬이 띄운 자식 프로세스가 실제로 죽었는지"를 PID 로 확인해야 한다. 그 PID 를
/// 외부에서 알아내는 유일한 길이 OS 프로세스 트리 열거다. Toolhelp32Snapshot 으로 전 프로세스를
/// 훑어 `th32ParentProcessID == parent` 인 항목의 PID 를 모은다.
///
/// best-effort: 스냅샷/순회 실패 시 빈 Vec. ppid 는 OS 가 즉시 갱신하지 않는 경우가 있어
/// (부모가 죽으면 ppid 가 stale 일 수 있음) "살아있는 부모의 직계 자식" 용도로만 신뢰한다.
#[cfg(windows)]
pub fn child_pids(parent: u32) -> Vec<u32> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    let mut out = Vec::new();
    if parent == 0 {
        return out;
    }
    // SAFETY: 전체 프로세스 스냅샷 생성. 실패면 빈 핸들 → 빈 결과.
    let snapshot = match unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) } {
        Ok(h) => h,
        Err(_) => return out,
    };

    let mut entry = PROCESSENTRY32W {
        dwSize: core::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };

    // SAFETY: 유효 스냅샷 핸들 + dwSize 가 채워진 entry. First 가 성공하면 Next 로 순회한다.
    let first = unsafe { Process32FirstW(snapshot, &mut entry) };
    if first.is_ok() {
        loop {
            if entry.th32ParentProcessID == parent {
                out.push(entry.th32ProcessID);
            }
            // SAFETY: 같은 유효 핸들 + entry. 더 없으면 Err → break.
            if unsafe { Process32NextW(snapshot, &mut entry) }.is_err() {
                break;
            }
        }
    }
    // SAFETY: CreateToolhelp32Snapshot 이 반환한 유효 핸들을 한 번만 닫는다.
    unsafe {
        let _ = CloseHandle(snapshot);
    }
    out
}

// ── non-windows stub ─────────────────────────────────────────────────────────────

#[cfg(not(windows))]
pub fn process_creation_time(_pid: u32) -> Option<u64> {
    None
}

/// non-windows: 프로세스 트리 열거 미구현(데몬은 Windows 1차) — 빈 목록.
#[cfg(not(windows))]
pub fn child_pids(_parent: u32) -> Vec<u32> {
    Vec::new()
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
    fn child_pids_parent_zero_is_empty() {
        // 부모 0(시스템 idle) 은 우리 관심 대상이 아님 → 빈 목록.
        assert!(child_pids(0).is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn child_pids_finds_spawned_child() {
        // 자기 자신이 자식을 띄우면 그 PID 가 child_pids(자기 PID) 에 나타나야 한다.
        // best-effort 헬퍼의 기본 동작(부모-자식 매칭)을 현재 프로세스 기준으로 실측.
        let mut child = std::process::Command::new("cmd.exe")
            .args(["/c", "ping -n 3 127.0.0.1 > NUL"]) // 잠깐 살아있는 자식
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("cmd.exe spawn");
        let child_pid = child.id();
        let me = std::process::id();

        // ppid 반영에 약간의 지연이 있을 수 있어 짧게 폴링.
        let mut found = false;
        for _ in 0..50 {
            if child_pids(me).contains(&child_pid) {
                found = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let _ = child.kill();
        let _ = child.wait();
        assert!(
            found,
            "spawn 한 자식 PID({child_pid}) 가 child_pids({me}) 에 나타나야"
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
