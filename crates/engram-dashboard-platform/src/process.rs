//! PID liveness · 프로세스 시작시각(creation time) 조회와 그 결과의 세 갈래(앎 · 사라짐 · 못 읽음) ·
//! 프로세스 표(pid, ppid) 한 장 · 자식 PID 열거 · 한 뿌리 아래 살아 있는 프로세스의 신원 목록([`subtree`]) ·
//! 한 프로세스 트리 끄기([`kill_tree`]).
//!
//! "그 PID 가 아직 그 프로세스인가" 를 판정하는 곳이 여러 crate 에 있다 — 예: `net`(portfile 의 stale
//! 판정) · 셸(`discovery` 모듈의 데몬 발견) · `daemon`(daemon.json 에 자기 시작시각 기록) · `agent`(codex
//! 자식의 신원 판정). 모두 판정 로직을 사본으로 갖지 않고 이 모듈을 본다 — 부르는 곳의 정본은
//! `rg "engram_dashboard_platform::process" crates src-tauri`. ★에이전트 런타임 안으로 되돌리지 말 것★ — 에이전트
//! 밖의 소비자가 이 함수들 때문에 런타임 전체를 의존하게 된다(ADR-0175 §맥락이 잰 그 상태).
//!
//! ★왜 creation time 까지 보나★: PID 는 OS 가 재사용한다. 데몬이 죽고 같은 PID 를 다른
//! 프로세스가 받으면 "PID 살아있음"만으로는 false-live(엉뚱한 프로세스를 데몬으로 오인)가
//! 난다. 그래서 "PID 살아있음 AND 그 PID 의 현재 creation time == 기록된 값"으로 판정해
//! PID 재사용을 직접 구분한다.

// ADR-0266

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

// ── 뿌리 아래 신원 목록 ──────────────────────────────────────────────────────────

/// 한 프로세스의 신원 — [`subtree`] 가 돌려주는 한 뿌리 아래의 프로세스 하나.
///
/// ★두 칸은 **함께** 대조하라고 있다★ — PID 는 OS 가 재사용하므로 PID 단독 일치는 남의 프로세스를
/// 그 프로세스로 본다(ADR-0218 결정 2). 그래서 이 타입에는 `pid` 만 꺼내 쓰는 헬퍼를 두지 않는다.
/// 모양이 같은 [`crate::file_holders::Holder`] 는 「한 파일을 연 프로세스」라 뜻이 다르다 — 합치지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessIdentity {
    pub pid: u32,
    /// 프로세스 생성 FILETIME — [`process_creation_time`] 이 돌려주는 값과 같은 척도다.
    pub start_time: u64,
}

/// `root` 와 그 아래 **살아 있는** 후손 전부의 신원.
///
/// - ★`root` 의 시작시각이 `root_start_time` 과 다르면 **빈 목록**이다★ — 그 PID 는 더는 호출자가 아는
///   그 프로세스가 아니다(죽었거나 재사용됐다). 뿌리의 신원을 확인하지 않고 걸으면 남의 프로세스 나무를
///   그 뿌리의 나무로 돌려주게 된다.
/// - **후손의 시작시각은 지금 읽어 채운다** — 미리 아는 값이 없기 때문이다. 그래서 돌려준 신원은 언제나
///   「이 순간 그 PID 인 프로세스」의 것이고, 열거와 대조 사이에 PID 가 재사용돼도 시작시각이 갈라서
///   걸러진다.
/// - ★**부모보다 먼저 태어난 항목은 후손이 아니다 — 버린다**★. 이것이 이 함수의 유일한 **의미** 규칙이다.
///   [`child_pids`] 의 ppid 는 부모가 죽은 뒤에도 그대로 남는다(그 doc 의 단서). 그래서 어떤 프로세스를 낳은
///   부모가 죽고 그 PID 가 뿌리나 그 후손에게 재사용되면, 남겨진 고아가 재사용된 PID 의 자식으로 열거된다.
///   그 고아는 재사용 **이전**에 — 곧 지금 그 PID 를 쥔 프로세스보다 **먼저** — 태어났으므로 이 비교가 그
///   경로를 전부 거른다. 뿌리 자신이 죽고 번호가 또 넘어간 경우는 첫 항목(뿌리 신원 확인)이 막는다.
///   ★같은 시각(`==`)은 통과시킨다★ — FILETIME 눈금 하나 안에서 뜬 부모·자식이 실재할 수 있고, 거기서
///   막으면 정상 자식을 잃는다. 막는 것은 **먼저 태어난** 것뿐이다.
/// - ★**이 규칙이 못 거르는 경우가 셋 있다 — 「닫혀 있다」고 읽지 말 것**★(받아들일지는 호출자가 정한다):
///   1. **열거와 시작시각 읽기 사이의 PID 재사용** — `child_pids` 가 준 번호의 주인이 읽기 전에 죽고
///      번호가 넘어가면, 읽는 시작시각은 새 주인의 것이다. 열거와 조회가 별개 syscall 이라 원자적으로
///      고칠 수단이 없다(창은 마이크로초 단위).
///   2. **같은 눈금(`==`)** — 위에서 통과시킨 그것.
///   3. **명시 부모 지정** — Windows 는 `PROC_THREAD_ATTRIBUTE_PARENT_PROCESS` 로 부모를 박아 프로세스를
///      만들 수 있다. 그렇게 태어난 남은 뿌리보다 **뒤**에 태어나고도 뿌리의 PID 를 ppid 로 달 수 있어
///      이 비교를 지난다.
/// - 시작시각을 못 읽는 항목은 **빼고, 그 아래로 내려가지도 않는다** — 신원의 절반이 없으면 대조가 PID
///   단독으로 내려앉고, 그 항목을 부모로 쓰면 위 순서 규칙을 적용할 기준이 없어 그 가지 전체가 무검증이 된다.
/// - `root_start_time` 이 0(미상)이면 빈 목록이다. 같은 사유.
/// - **best-effort**: 열거가 실패하면 그만큼 덜 돌려주고 오류는 올리지 않는다 — 짧은 목록은 「그 아래에는
///   없다」의 확증이 아니다.
/// - ★깊이 제한을 두지 않는다 — 대신 **이미 본 신원을 다시 안 내려간다**★. ppid 는 OS 가 즉시 갱신하지
///   않아 순환처럼 보이는 모양이 나올 수 있고, 그때 방문 표시가 없으면 이 함수가 안 끝난다.
///   ★표시의 키는 PID 가 아니라 **신원**이다★ — PID 로만 표시하면 재사용된 PID 가 「이미 봤다」로 건너뛰어,
///   같은 번호를 쓰는 **다른** 프로세스가 통째로 안 보인다.
pub fn subtree(root: u32, root_start_time: u64) -> Vec<ProcessIdentity> {
    if root == 0 || root_start_time == 0 {
        return Vec::new();
    }
    if process_creation_time(root) != Some(root_start_time) {
        return Vec::new();
    }

    walk(
        ProcessIdentity {
            pid: root,
            start_time: root_start_time,
        },
        &|pid| {
            child_pids(pid)
                .into_iter()
                .filter_map(|child| {
                    process_creation_time(child).map(|start_time| ProcessIdentity {
                        pid: child,
                        start_time,
                    })
                })
                .collect()
        },
    )
}

/// [`subtree`] 의 **규칙만** — OS 는 `children` 뒤에 있다(ADR-0012). 규칙 둘(부모보다 먼저 태어난 것은
/// 버린다 · 이미 본 **신원**은 다시 안 내려간다)의 사유 정본은 [`subtree`] 의 doc 이고 여기 되풀어
/// 적지 않는다.
///
/// `children` 은 **신원을 못 읽은 항목을 이미 걸러서** 준다 — 그 거르기가 이 규칙 밖인 것은, 신원 없는
/// 항목에는 적용할 순서 기준 자체가 없기 때문이다.
fn walk(
    root: ProcessIdentity,
    children: &dyn Fn(u32) -> Vec<ProcessIdentity>,
) -> Vec<ProcessIdentity> {
    let mut out = vec![root];
    let mut visited = vec![root];
    let mut frontier = vec![root];
    while let Some(parent) = frontier.pop() {
        for child in children(parent.pid) {
            if child.start_time < parent.start_time {
                continue;
            }
            if visited.contains(&child) {
                continue;
            }
            visited.push(child);
            frontier.push(child);
            out.push(child);
        }
    }
    out
}

/// `pid` 와 그 자손 전부를 강제로 끝낸다 — Windows = `taskkill /PID <pid> /F /T`. 끝내기를 맡기기만 하고 기다리지
/// 않는다. taskkill 의 종료 코드는 보지 않는다 — 이미 끝난 프로세스에도 0 이 아닌 코드(128)로 답한다. 그래서
/// `Ok` 는 「끝났다」가 아니라 「taskkill 을 돌렸다」다.
///
/// `Err` = taskkill 을 띄우지 못했다(종류 `Other` · 문구에 그 사유가 든다). Windows 밖 = `Unsupported` — 이
/// 수단이 없다. ★Windows 에서는 `Unsupported` 가 나오지 않는다★ — 부르는 쪽이 그 종류 하나로 「이 OS 에서는 못
/// 한다」를 가른다.
// ADR-0266
pub fn kill_tree(pid: u32) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::process::{Command, Stdio};
        let mut cmd = Command::new("taskkill");
        cmd.args(["/PID", &pid.to_string(), "/F", "/T"])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        crate::spawn::hide_console_window(&mut cmd);
        cmd.status()
            .map(drop)
            .map_err(|e| std::io::Error::other(format!("taskkill 실행 실패: {e}")))
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        Err(std::io::ErrorKind::Unsupported.into())
    }
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

    // ── 뿌리 아래 신원 목록(subtree) ─────────────────────────────────────────────────

    const ROOT: ProcessIdentity = ProcessIdentity {
        pid: 4242,
        start_time: 1_000,
    };

    fn id(pid: u32, start_time: u64) -> ProcessIdentity {
        ProcessIdentity { pid, start_time }
    }

    /// 부모 PID → 그 아래 신원들. 표에 없으면 자식 없음.
    fn table(rows: &[(u32, Vec<ProcessIdentity>)]) -> impl Fn(u32) -> Vec<ProcessIdentity> + '_ {
        move |pid| {
            rows.iter()
                .find(|(parent, _)| *parent == pid)
                .map(|(_, kids)| kids.clone())
                .unwrap_or_default()
        }
    }

    // ── 순서 규칙(G1) ────────────────────────────────────────────────────────────

    /// ★이 항목이 지키는 것 = 「남의 고아가 재사용된 PID 를 타고 나무에 들어오지 않는다」★.
    /// 그 고아는 지금 그 PID 를 쥔 부모보다 **먼저** 떠 있었으므로 시작시각이 앞선다 — 그것이 유일한
    /// 구분 표식이다.
    #[test]
    fn a_child_that_predates_its_parent_is_not_ours() {
        let stranger = id(777, ROOT.start_time - 1);
        let got = walk(ROOT, &table(&[(ROOT.pid, vec![stranger])]));
        assert_eq!(
            got,
            vec![ROOT],
            "부모보다 먼저 태어난 항목이 들어왔다: {got:?}"
        );
    }

    /// 그 남 아래 매달린 것까지 통째로 안 들어온다 — 가지째 끊는다.
    #[test]
    fn nothing_under_a_predating_child_gets_in_either() {
        let stranger = id(777, ROOT.start_time - 1);
        let grandchild = id(778, ROOT.start_time + 5);
        let got = walk(
            ROOT,
            &table(&[(ROOT.pid, vec![stranger]), (stranger.pid, vec![grandchild])]),
        );
        assert_eq!(got, vec![ROOT], "끊긴 가지 아래가 새 들어왔다: {got:?}");
    }

    /// 같은 눈금에 뜬 자식은 정상이다 — 막는 것은 **먼저** 태어난 것뿐이다.
    #[test]
    fn a_child_born_on_the_same_tick_is_ours() {
        let twin = id(4816, ROOT.start_time);
        let got = walk(ROOT, &table(&[(ROOT.pid, vec![twin])]));
        assert_eq!(got, vec![ROOT, twin]);
    }

    #[test]
    fn the_walk_reaches_grandchildren() {
        let child = id(4816, ROOT.start_time + 1);
        let grandchild = id(19_468, ROOT.start_time + 2);
        let got = walk(
            ROOT,
            &table(&[(ROOT.pid, vec![child]), (child.pid, vec![grandchild])]),
        );
        assert_eq!(got, vec![ROOT, child, grandchild]);
    }

    // ── 방문 표시(G2) ────────────────────────────────────────────────────────────

    /// ★표시가 PID 키면 재사용된 번호가 「이미 봤다」로 건너뛰어 **다른** 프로세스가 통째로 사라진다★.
    #[test]
    fn a_recycled_pid_is_a_different_process_not_a_repeat() {
        let first = id(4816, ROOT.start_time + 1);
        let recycled = id(4816, ROOT.start_time + 9);
        let got = walk(
            ROOT,
            &table(&[(ROOT.pid, vec![first]), (first.pid, vec![recycled])]),
        );
        assert!(
            got.contains(&recycled),
            "같은 번호의 다른 프로세스가 표시에 먹혔다: {got:?}"
        );
    }

    /// stale ppid 가 순환처럼 보여도 끝난다 — 같은 신원을 두 번 안 내려간다.
    #[test]
    fn a_cycle_shaped_table_terminates() {
        let a = id(10, ROOT.start_time + 1);
        let b = id(11, ROOT.start_time + 1);
        let got = walk(
            ROOT,
            &table(&[(ROOT.pid, vec![a]), (a.pid, vec![b]), (b.pid, vec![a, b])]),
        );
        assert_eq!(got.len(), 3, "{got:?}");
    }

    #[test]
    fn an_unknown_root_has_no_subtree() {
        assert!(subtree(0, 1).is_empty());
        assert!(
            subtree(std::process::id(), 0).is_empty(),
            "시작시각 미상이면 남는 것이 PID 단독 대조다"
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_wrong_start_time_disowns_the_root() {
        let me = std::process::id();
        let start = process_creation_time(me).expect("자기 creation time 조회 가능");
        assert!(
            subtree(me, start.wrapping_add(1)).is_empty(),
            "시작시각이 어긋난 PID 를 그 뿌리로 봤다"
        );
    }

    /// 뿌리 자신뿐 아니라 **후손이 목록에 들어오는 것**을 실물 나무(`cmd.exe` → `ping`)로 잰다 — 손자가
    /// 안 잡히면 뿌리 하나만 대조하는 것과 같아진다.
    #[cfg(windows)]
    #[test]
    fn the_subtree_reaches_past_the_root() {
        let mut child = std::process::Command::new("cmd.exe")
            .args(["/c", "ping", "-n", "4", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("cmd.exe 기동");
        let root = child.id();
        let start = process_creation_time(root).expect("자식 creation time 조회 가능");

        // 손자(`ping`)가 뜰 때까지 짧게 기다린다 — cmd 가 먼저 뜨고 그다음에 띄운다.
        let mut found = Vec::new();
        for _ in 0..40 {
            found = subtree(root, start);
            if found.len() > 1 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        let _ = child.kill();
        let _ = child.wait();

        assert!(
            found.iter().any(|p| p.pid == root),
            "뿌리 자신이 목록에 없다: {found:?}"
        );
        assert!(
            found.len() > 1,
            "후손이 하나도 안 잡혔다 — 뿌리 PID 만 돌려준 셈이다: {found:?}"
        );
        assert!(
            found.iter().all(|p| p.start_time != 0),
            "신원의 절반이 빈 항목이 섞였다: {found:?}"
        );
    }
}
