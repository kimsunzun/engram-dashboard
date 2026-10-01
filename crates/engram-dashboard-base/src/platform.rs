//! PID liveness · 프로세스 시작시각(creation time) 조회와 그 결과의 세 갈래(앎 · 사라짐 · 못 읽음) ·
//! 프로세스 표(pid, ppid) 한 장.
//!
//! ★왜 바닥 crate 인가★: "그 PID 가 아직 그 프로세스인가" 를 판정하는 곳이 셋이다 — `net`(portfile 의
//! stale 판정) · `discovery`(데몬 발견) · `daemon`(daemon.json 에 자기 시작시각 기록). 셋 다 판정 로직을
//! 사본으로 갖지 않으려면 한 곳을 봐야 하는데, 그 한 곳이 에이전트 런타임 안이면 셋이 런타임 전체를
//! 의존하게 된다(ADR-0175 §맥락이 잰 그 상태). 여기는 도메인 지식이 0 이고 소비자가 여럿이라 입주 조건을
//! 채운다.
//!
//! ★왜 creation time 까지 보나★: PID 는 OS 가 재사용한다. 데몬이 죽고 같은 PID 를 다른
//! 프로세스가 받으면 "PID 살아있음"만으로는 false-live(엉뚱한 프로세스를 데몬으로 오인)가
//! 난다. 그래서 "PID 살아있음 AND 그 PID 의 현재 creation time == 기록된 값"으로 판정해
//! PID 재사용을 직접 구분한다.

/// 한 PID 의 시작시각을 물은 결과 — 「못 읽었다」를 한 갈래로 뭉개지 않는다.
///
/// - `Known` = 그 PID 인 프로세스의 생성 시각. 척도는 [`process_creation_time`] 과 같다. ★끝났어도 누가 그
///   핸들을 쥐고 있으면(프로세스 객체가 남아 있으면) `Known` 이다★ — 「살아 있다」가 아니다.
/// - `Gone` = 그 번호의 프로세스 객체가 지금 없다(여는 실패가 87 일 때만).
/// - `Unknown` = 있는지 없는지 모른다 — 열 권한이 없거나 그 밖의 이유로 못 열었거나 시각 조회가 실패했다.
///   PID 0 과 이 수단이 없는 OS 도 여기다.
// ADR-0257
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessStart {
    Known(u64),
    Gone,
    Unknown,
}

/// `Gone` 은 확실한 부재 신호에만 선다 — 여는 실패의 판정은 `pid_alive` 의 것과 같은 하나다(87 만 부재).
// ADR-0257
#[cfg(windows)]
pub fn process_start(pid: u32) -> ProcessStart {
    use windows::Win32::Foundation::{CloseHandle, FILETIME};
    use windows::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    // System Idle Process 는 살아 있는데 `OpenProcess` 가 부재와 같은 87 로 실패한다 — 열면 `Gone` 으로 읽힌다.
    if pid == 0 {
        return ProcessStart::Unknown;
    }
    // SAFETY: 최소 권한으로 PID 핸들 open.
    let handle = match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
        Ok(h) => h,
        Err(e) if alive_from_open_error(win32_from_hresult(e.code().0)) => {
            return ProcessStart::Unknown
        }
        Err(_) => return ProcessStart::Gone,
    };

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
        return ProcessStart::Unknown;
    }
    ProcessStart::Known(((creation.dwHighDateTime as u64) << 32) | (creation.dwLowDateTime as u64))
}

/// u64 = GetProcessTimes lpCreationTime(FILETIME — 1601-01-01 UTC 부터 100나노초 간격 수)의
/// high/low 32비트를 합친 값. 같은 프로세스면 불변, 재사용 PID 면 다르다.
pub fn process_creation_time(pid: u32) -> Option<u64> {
    match process_start(pid) {
        ProcessStart::Known(start) => Some(start),
        ProcessStart::Gone | ProcessStart::Unknown => None,
    }
}

/// 데몬이 daemon.json 에 기록할 값.
#[cfg(windows)]
pub fn current_process_start_time() -> Option<u64> {
    process_creation_time(std::process::id())
}

/// windows-rs 의 `OpenProcess` 실패는 `windows::core::Error` 이고, 그 `.code().0` 은
/// `HRESULT_FROM_WIN32` 로 래핑된 HRESULT 다(예: INVALID_PARAMETER=0x80070057,
/// ACCESS_DENIED=0x80070005). FACILITY_WIN32(0x7) HRESULT 의 하위 16비트가 원래 win32
/// 코드이므로 `hr & 0xFFFF` 로 되돌린다. (facility 가 win32 가 아니면 의미 없는 값이 나오나,
/// OpenProcess 실패는 사실상 win32 facility 라 호출부 분류에서 보수 기본값으로 흡수된다.)
#[cfg(windows)]
fn win32_from_hresult(hr: i32) -> u32 {
    (hr as u32) & 0xFFFF
}

/// ★왜 코드별 분기인가(false-live 버그 맥락)★: 기존엔 OpenProcess 실패를 무조건 live 로
/// 봤다(`Err(_) => true`). 그 결과 "그런 프로세스 없음"(ERROR_INVALID_PARAMETER)도 살아있다고
/// 오판 → 죽은 데몬에 앱이 붙으려다 'reconnecting' 고착(직전 세션 실제 발생). 부재와 권한부족을
/// 구분해야 한다:
///   - ERROR_INVALID_PARAMETER(87) = 그 PID 의 프로세스 자체가 없음 → dead(false).
///   - ERROR_ACCESS_DENIED(5) = 프로세스는 존재하나 열 권한이 없음 → live(true).
///   - 그 외 실패 = 원인 불명 → 보수적으로 live(true)(오탐보다 안전: 산 프로세스를 죽었다고
///     오판하면 멀쩡한 데몬을 버리게 되므로).
///
/// ★왜 부재를 87 하나로만 좁혔나(의도적 비대칭)★: 죽은 PID 의 OpenProcess 실패가 항상 87 이라는
/// MS 문서 보장은 없다(예: 방금 종료된 zombie·권한경계는 다른 코드 가능, ERROR_NOT_FOUND(1168) 등).
/// 추측으로 dead 코드를 늘리면 산 데몬을 죽었다고 오판(false-dead)할 위험이 생기므로, "확실한 부재
/// 시그널(87)만 dead, 나머지는 안전하게 live" 로 좁혔다. 빠진 부재 코드는 false-live 로 남지만, 이는
/// 옛 버그의 부분 재현일 뿐 새 false-dead 를 만들지 않으며, 상위 `pid_alive_with_start_time` 의
/// creation-time 대조가 2차 방어선이라 영구 교착까진 가지 않는다. 실측으로 부재 코드가 더 확인되면
/// 그때 추가한다(추측 추가 금지 — 명세 오염).
#[cfg(windows)]
fn alive_from_open_error(win32_code: u32) -> bool {
    // windows crate 의 ERROR_INVALID_PARAMETER.0 / ERROR_ACCESS_DENIED.0 과 동일한 리터럴.
    const ERROR_ACCESS_DENIED: u32 = 5;
    const ERROR_INVALID_PARAMETER: u32 = 87;
    match win32_code {
        ERROR_INVALID_PARAMETER => false,
        ERROR_ACCESS_DENIED => true,
        _ => true,
    }
}

/// creation time 을 무시하는 형제 — start_time 미상(0)일 때의 보수 fallback 으로만 쓴다.
#[cfg(windows)]
pub fn pid_alive(pid: u32) -> bool {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    if pid == 0 {
        return false;
    }
    // SAFETY: 최소 권한으로 PID 핸들 open.
    let handle = match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
        Ok(h) => h,
        Err(e) => return alive_from_open_error(win32_from_hresult(e.code().0)),
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
                true
            } else {
                actual == expected_start
            }
        }
        None => {
            if expected_start == 0 {
                pid_alive(pid)
            } else {
                false
            }
        }
    }
}

/// ★왜 필요한가★: AgentInfo/WS 프로토콜은 PTY child 의 PID 를 노출하지 않는다(설계상 손발/두뇌
/// 분리 — 프론트는 PID 를 몰라도 된다). 그러나 실프로세스 격리테스트(데몬 .exe kill → PTY child
/// 동반 사망)는 "데몬이 띄운 자식 프로세스가 실제로 죽었는지"를 PID 로 확인해야 한다. 그 PID 를
/// 외부에서 알아내는 유일한 길이 OS 프로세스 트리 열거다.
///
/// best-effort: 스냅샷/순회 실패 시 빈 Vec. ppid 는 OS 가 즉시 갱신하지 않는 경우가 있어
/// (부모가 죽으면 ppid 가 stale 일 수 있음) "살아있는 부모의 직계 자식" 용도로만 신뢰한다.
pub fn child_pids(parent: u32) -> Vec<u32> {
    if parent == 0 {
        return Vec::new();
    }
    process_parent_table()
        .unwrap_or_default()
        .into_iter()
        .filter(|&(_, ppid)| ppid == parent)
        .map(|(pid, _)| pid)
        .collect()
}

/// 지금 떠 있는 프로세스 전부의 `(pid, ppid)` — Toolhelp 스냅숏 **한 장**.
///
/// `None` = 목록을 끝까지 못 읽었다(스냅숏 생성 · 첫 항목 · 순회 중 「끝」이 아닌 오류). ★잘린 목록을 돌려주지
/// 않는다★ — 빠진 줄은 호출자에게 「그 프로세스가 없다」로 읽힌다. ppid 의 단서는 [`child_pids`] 와 같다.
// ADR-0257
#[cfg(windows)]
pub fn process_parent_table() -> Option<Vec<(u32, u32)>> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    // SAFETY: 전체 프로세스 스냅샷 생성.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }.ok()?;

    let mut entry = PROCESSENTRY32W {
        dwSize: core::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut read_all = || {
        // SAFETY: 유효 스냅샷 핸들 + dwSize 가 채워진 entry.
        unsafe { Process32FirstW(snapshot, &mut entry) }.ok()?;
        let mut rows = Vec::new();
        loop {
            rows.push((entry.th32ProcessID, entry.th32ParentProcessID));
            // SAFETY: 같은 유효 핸들 + entry.
            match unsafe { Process32NextW(snapshot, &mut entry) } {
                Ok(()) => {}
                Err(e) if list_ended(e.code().0) => return Some(rows),
                Err(_) => return None,
            }
        }
    };
    let table = read_all();
    // SAFETY: CreateToolhelp32Snapshot 이 반환한 유효 핸들을 한 번만 닫는다.
    unsafe {
        let _ = CloseHandle(snapshot);
    }
    table
}

/// `Process32NextW` 의 실패 하나가 목록의 끝인가. windows-rs 는 그 실패를 `HRESULT_FROM_WIN32(GetLastError())`
/// 로 싸서 주고, 끝은 `ERROR_NO_MORE_FILES`(18) 하나뿐이다 — 그 밖은 목록이 도중에 잘렸다는 뜻이다.
#[cfg(any(windows, test))]
fn list_ended(hr: i32) -> bool {
    const HRESULT_FROM_ERROR_NO_MORE_FILES: i32 = 0x8007_0012_u32 as i32;
    hr == HRESULT_FROM_ERROR_NO_MORE_FILES
}

// ── non-windows stub ─────────────────────────────────────────────────────────────

#[cfg(not(windows))]
pub fn process_start(_pid: u32) -> ProcessStart {
    ProcessStart::Unknown
}

/// non-windows: 프로세스 트리 열거 미구현(데몬은 Windows 1차).
#[cfg(not(windows))]
pub fn process_parent_table() -> Option<Vec<(u32, u32)>> {
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
        assert!(!pid_alive(0));
        assert!(!pid_alive_with_start_time(0, 0));
        assert!(!pid_alive_with_start_time(0, 12345));
        assert_eq!(process_creation_time(0), None);
    }

    #[cfg(windows)]
    #[test]
    fn current_process_is_alive_with_matching_start_time() {
        let pid = std::process::id();
        let start = current_process_start_time().expect("자기 creation time 조회 가능");
        assert!(start != 0, "creation time 은 0 이 아니어야 함");
        assert!(pid_alive_with_start_time(pid, start), "자기 자신은 live");
        assert!(pid_alive(pid), "자기 자신은 OpenProcess 로도 live");
    }

    #[cfg(windows)]
    #[test]
    fn current_pid_with_wrong_start_time_is_dead() {
        let pid = std::process::id();
        let real = current_process_start_time().unwrap();
        let wrong = real.wrapping_add(1);
        assert!(
            !pid_alive_with_start_time(pid, wrong),
            "creation time 불일치면 dead(PID 재사용 방어)"
        );
    }

    #[cfg(windows)]
    #[test]
    fn child_pids_parent_zero_is_empty() {
        assert!(child_pids(0).is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn child_pids_finds_spawned_child() {
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

    // ── 프로세스 표 · 시작시각 세 갈래(ADR-0257) ──────────────────────────────────────

    #[test]
    fn only_no_more_files_ends_the_list() {
        assert!(list_ended(0x8007_0012_u32 as i32));
        assert!(!list_ended(0x8007_0005_u32 as i32), "ACCESS_DENIED = 잘림");
        assert!(!list_ended(0x8007_0018_u32 as i32), "BAD_LENGTH = 잘림");
        assert!(!list_ended(0), "GetLastError 가 비어 있던 실패 = 잘림");
    }

    #[cfg(windows)]
    #[test]
    fn the_end_code_is_the_shape_windows_rs_gives() {
        use windows::core::HRESULT;
        use windows::Win32::Foundation::ERROR_NO_MORE_FILES;
        assert!(list_ended(HRESULT::from_win32(ERROR_NO_MORE_FILES.0).0));
    }

    #[test]
    fn pid_zero_start_is_unknown_not_gone() {
        assert_eq!(process_start(0), ProcessStart::Unknown);
    }

    #[cfg(windows)]
    #[test]
    fn the_table_lists_a_spawned_child_with_its_parent() {
        let mut child = std::process::Command::new("cmd.exe")
            .args(["/c", "ping -n 3 127.0.0.1 > NUL"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("cmd.exe spawn");
        let row = (child.id(), std::process::id());

        let mut found = false;
        for _ in 0..50 {
            let table = process_parent_table().expect("스냅숏 한 장");
            if table.contains(&row) {
                found = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let _ = child.kill();
        let _ = child.wait();
        assert!(found, "띄운 자식의 줄 {row:?} 이 표에 없다");
    }

    #[cfg(windows)]
    #[test]
    fn an_exited_pid_starts_as_gone() {
        let mut child = std::process::Command::new("cmd.exe")
            .args(["/c", "exit"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("cmd.exe spawn");
        let pid = child.id();
        // 우리 핸들이 프로세스 객체를 붙들고 있어 끝났어도 읽힌다.
        let ProcessStart::Known(start) = process_start(pid) else {
            panic!("쥔 자식의 시작시각을 못 읽었다");
        };
        let _ = child.wait();
        drop(child);

        // 다른 핸들(콘솔 호스트 등)이 늦게 놓을 수 있어 옛 신원이 사라질 때까지 기다린다.
        let mut now = process_start(pid);
        for _ in 0..100 {
            if now != ProcessStart::Known(start) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
            now = process_start(pid);
        }
        match now {
            ProcessStart::Gone => {}
            // 번호가 벌써 남에게 넘어갔다 — 옛 신원이 사라진 것은 같다(PID 재사용은 막을 수 없다).
            ProcessStart::Known(other) if other != start => {}
            other => panic!("끝난 PID {pid} 가 {other:?} 로 읽혔다"),
        }
    }

    #[cfg(windows)]
    #[test]
    fn unknown_start_time_falls_back_to_pid_liveness() {
        let pid = std::process::id();
        assert!(
            pid_alive_with_start_time(pid, 0),
            "미상이면 PID 생존으로 보수 판정"
        );
    }

    // ── OpenProcess 에러코드 → 생존 판정(순수, 실프로세스 비의존) ────────────────────

    #[cfg(windows)]
    #[test]
    fn alive_from_open_error_classifies_codes() {
        assert!(!alive_from_open_error(87), "부재 → dead");
        assert!(alive_from_open_error(5), "권한부족이나 존재 → live");
        assert!(alive_from_open_error(0), "불명 → 보수적 live");
        assert!(alive_from_open_error(1234), "불명 → 보수적 live");
    }

    #[cfg(windows)]
    #[test]
    fn win32_from_hresult_extracts_low_word() {
        assert_eq!(win32_from_hresult(0x8007_0057u32 as i32), 87); // INVALID_PARAMETER
        assert_eq!(win32_from_hresult(0x8007_0005u32 as i32), 5); // ACCESS_DENIED
    }
}
