//! PID liveness · process start time (creation time) query and its three outcomes (known, gone, unreadable) ·
//! one process table (pid, ppid) · child PID enumeration · the list of identities of live processes under one root ([`subtree`]) ·
//! killing one process tree ([`kill_tree`]).
//!
//! The place that decides "is that PID still that process" exists in several crates — e.g. `net` (stale
//! decision for the portfile) · the shell (daemon discovery in the `discovery` module) · `daemon` (records its own start time in daemon.json) · `agent` (identity
//! decision for the codex child). All of them look at this module instead of keeping a copy of the decision logic — the source of truth for the callers is
//! `rg "engram_dashboard_platform::process" crates src-tauri`. ★Do not move it back into the agent runtime★ — consumers
//! outside the agent would end up depending on the whole runtime because of these functions (the state ADR-0175 §Context measured).
//!
//! ★Why look at creation time too★: the OS reuses PIDs. If the daemon dies and another
//! process gets the same PID, "PID alive" alone gives a false-live (a wrong process mistaken for the daemon).
//! So it decides with "PID alive AND the PID's current creation time == the recorded value", which
//! tells PID reuse apart directly.

// ADR-0266

/// The result of asking one PID's start time — 「못 읽었다」 (could not read) is not lumped into a single branch.
///
/// - `Known` = the creation time of the process with that PID. The scale is the same as [`process_creation_time`]. ★Even if it has ended, if someone still holds that
///   handle (the process object remains) it is `Known`★ — it does not mean 「살아 있다」 (alive).
/// - `Gone` = there is no process object with that number now (only when the open failure is 87).
/// - `Unknown` = we do not know whether it exists — it could not be opened for lack of rights or another reason, or the time query failed.
///   PID 0 and an OS without this means also land here.
// ADR-0257
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessStart {
    Known(u64),
    Gone,
    Unknown,
}

/// `Gone` stands only on a certain absence signal — the decision of the open failure is the same single one as in `pid_alive` (only 87 is absence).
// ADR-0257
#[cfg(windows)]
pub fn process_start(pid: u32) -> ProcessStart {
    use windows::Win32::Foundation::{CloseHandle, FILETIME};
    use windows::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    // The System Idle Process is alive but `OpenProcess` fails with 87, the same as absence — opening it would read as `Gone`.
    if pid == 0 {
        return ProcessStart::Unknown;
    }
    // SAFETY: open the PID handle with minimum rights.
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
    // SAFETY: the valid handle just opened + four FILETIME output pointers on the stack.
    let ok = unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) }
        .is_ok();
    // SAFETY: close the valid handle once.
    unsafe {
        let _ = CloseHandle(handle);
    }
    if !ok {
        return ProcessStart::Unknown;
    }
    ProcessStart::Known(((creation.dwHighDateTime as u64) << 32) | (creation.dwLowDateTime as u64))
}

/// u64 = the high/low 32 bits of GetProcessTimes lpCreationTime (FILETIME — count of 100-nanosecond intervals since 1601-01-01 UTC)
/// joined into one value. Invariant for the same process, different for a reused PID.
pub fn process_creation_time(pid: u32) -> Option<u64> {
    match process_start(pid) {
        ProcessStart::Known(start) => Some(start),
        ProcessStart::Gone | ProcessStart::Unknown => None,
    }
}

/// The value the daemon records in daemon.json.
#[cfg(windows)]
pub fn current_process_start_time() -> Option<u64> {
    process_creation_time(std::process::id())
}

/// A windows-rs `OpenProcess` failure is a `windows::core::Error`, and its `.code().0` is
/// an HRESULT wrapped by `HRESULT_FROM_WIN32` (e.g. INVALID_PARAMETER=0x80070057,
/// ACCESS_DENIED=0x80070005). The low 16 bits of a FACILITY_WIN32 (0x7) HRESULT are the original win32
/// code, so it is recovered with `hr & 0xFFFF`. (If the facility is not win32 a meaningless value comes out, but
/// OpenProcess failures are in practice win32 facility, so the caller's classification absorbs it into the conservative default.)
#[cfg(windows)]
fn win32_from_hresult(hr: i32) -> u32 {
    (hr as u32) & 0xFFFF
}

/// ★Why branch per code (false-live bug context)★: previously every OpenProcess failure was
/// treated as live (`Err(_) => true`). As a result "no such process" (ERROR_INVALID_PARAMETER) was also
/// misjudged as alive → the app tried to attach to a dead daemon and got stuck on 'reconnecting' (actually happened in the previous session). Absence and lack of rights
/// must be told apart:
///   - ERROR_INVALID_PARAMETER(87) = there is no process with that PID at all → dead(false).
///   - ERROR_ACCESS_DENIED(5) = the process exists but we lack the right to open it → live(true).
///   - any other failure = cause unknown → conservatively live(true) (safer than a false alarm: judging a live process
///     dead would throw away a perfectly good daemon).
///
/// ★Why absence is narrowed to 87 alone (intentional asymmetry)★: there is no MS documentation guarantee that a dead PID's OpenProcess failure is always 87
/// (e.g. a just-terminated zombie or a permission boundary may give other codes, ERROR_NOT_FOUND(1168) etc.).
/// Adding dead codes by guesswork risks misjudging a live daemon as dead (false-dead), so it is narrowed to "only the certain absence
/// signal (87) is dead, the rest safely live". Absence codes left out remain false-live, but that is only a partial reproduction of the
/// old bug and creates no new false-dead, and the creation-time comparison in the upper `pid_alive_with_start_time` is a second line of defense,
/// so it does not reach a permanent deadlock. If measurement confirms more absence codes,
/// add them then (no adding by guesswork — spec poisoning).
#[cfg(windows)]
fn alive_from_open_error(win32_code: u32) -> bool {
    // Literals identical to the windows crate's ERROR_INVALID_PARAMETER.0 / ERROR_ACCESS_DENIED.0.
    const ERROR_ACCESS_DENIED: u32 = 5;
    const ERROR_INVALID_PARAMETER: u32 = 87;
    match win32_code {
        ERROR_INVALID_PARAMETER => false,
        ERROR_ACCESS_DENIED => true,
        _ => true,
    }
}

/// The sibling that ignores creation time — used only as the conservative fallback when start_time is unknown (0).
#[cfg(windows)]
pub fn pid_alive(pid: u32) -> bool {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    if pid == 0 {
        return false;
    }
    // SAFETY: open the PID handle with minimum rights.
    let handle = match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
        Ok(h) => h,
        Err(e) => return alive_from_open_error(win32_from_hresult(e.code().0)),
    };
    const STILL_ACTIVE: u32 = 259;
    let mut code: u32 = 0;
    // SAFETY: the valid handle just opened + an output pointer on the stack.
    let ok = unsafe { GetExitCodeProcess(handle, &mut code) }.is_ok();
    // SAFETY: close the valid handle once.
    unsafe {
        let _ = CloseHandle(handle);
    }
    if !ok {
        return true; // Query failed → conservatively alive.
    }
    code == STILL_ACTIVE
}

/// PID liveness decision (including the start-time comparison). true = alive (the very process we are looking for).
///
/// Decision rules (M2 PID-reuse defense):
///   - pid==0 → death (false). The system idle process cannot be our daemon.
///   - expected_start==0 (unknown, old daemon.json compatibility) → the creation time cannot be compared, so
///     **decide conservatively by PID liveness alone** (if alive, live). Do not recklessly label an old file stale.
///   - expected_start!=0 → live only when the PID's current creation time matches expected_start exactly.
///     A mismatch (reused PID) or a query failure (process absent) is dead.
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

/// ★Why it is needed★: the AgentInfo/WS protocol does not expose the PTY child's PID (by design the hands/brain
/// are separated — the frontend need not know PIDs). But the real-process isolation test (kill the daemon .exe → the PTY child
/// dies with it) must confirm by PID "whether the child process the daemon launched actually died". The only way to learn that PID
/// from outside is enumerating the OS process tree.
///
/// best-effort: on a snapshot/walk failure, an empty Vec. The OS sometimes does not update ppid immediately
/// (when the parent dies ppid may be stale), so it is trusted only for the purpose of "direct children of a live parent".
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

/// `(pid, ppid)` of all processes currently running — **one** Toolhelp snapshot.
///
/// `None` = the list could not be read to the end (snapshot creation, first entry, an error during the walk that is not 「끝」 (end)). ★It does not return a truncated
/// list★ — a missing line would read to the caller as 「그 프로세스가 없다」 (that process does not exist). The ppid caveat is the same as for [`child_pids`].
// ADR-0257
#[cfg(windows)]
pub fn process_parent_table() -> Option<Vec<(u32, u32)>> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    // SAFETY: create a snapshot of all processes.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }.ok()?;

    let mut entry = PROCESSENTRY32W {
        dwSize: core::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut read_all = || {
        // SAFETY: a valid snapshot handle + an entry with dwSize filled in.
        unsafe { Process32FirstW(snapshot, &mut entry) }.ok()?;
        let mut rows = Vec::new();
        loop {
            rows.push((entry.th32ProcessID, entry.th32ParentProcessID));
            // SAFETY: the same valid handle + entry.
            match unsafe { Process32NextW(snapshot, &mut entry) } {
                Ok(()) => {}
                Err(e) if list_ended(e.code().0) => return Some(rows),
                Err(_) => return None,
            }
        }
    };
    let table = read_all();
    // SAFETY: close the valid handle that CreateToolhelp32Snapshot returned, exactly once.
    unsafe {
        let _ = CloseHandle(snapshot);
    }
    table
}

/// Is a single failure of `Process32NextW` the end of the list. windows-rs wraps that failure as `HRESULT_FROM_WIN32(GetLastError())`
/// and the end is the single `ERROR_NO_MORE_FILES`(18) — anything else means the list was cut off midway.
#[cfg(any(windows, test))]
fn list_ended(hr: i32) -> bool {
    const HRESULT_FROM_ERROR_NO_MORE_FILES: i32 = 0x8007_0012_u32 as i32;
    hr == HRESULT_FROM_ERROR_NO_MORE_FILES
}

// ── identity list under a root ──────────────────────────────────────────────────────────

/// The identity of one process — one process under a root as returned by [`subtree`].
///
/// ★The two fields are there to be compared **together**★ — the OS reuses PIDs, so matching on PID alone takes someone else's process
/// for that process (ADR-0218 decision 2). So this type has no helper that takes out only `pid`.
/// [`crate::file_holders::Holder`] has the same shape but means 「한 파일을 연 프로세스」 (a process that has a file open), so the two differ — do not merge them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessIdentity {
    pub pid: u32,
    /// Process creation FILETIME — the same scale as the value [`process_creation_time`] returns.
    pub start_time: u64,
}

/// The identities of `root` and all **live** descendants under it.
///
/// - ★If `root`'s start time differs from `root_start_time`, it is an **empty list**★ — that PID is no longer the
///   process the caller knows (it died or was reused). Walking without confirming the root's identity would return someone else's process tree
///   as that root's tree.
/// - **Descendants' start times are read and filled in now** — because there is no value known beforehand. So the identity returned is always
///   that of 「이 순간 그 PID 인 프로세스」 (the process that holds that PID at this moment), and even if the PID is reused between enumeration and comparison, the start time differs and
///   it is filtered out.
/// - ★**An entry born before its parent is not a descendant — discard it**★. This is the only **semantic** rule of this function.
///   The ppid in [`child_pids`] stays as it was even after the parent dies (the caveat in that doc). So if the parent that spawned some process
///   dies and that PID is reused by the root or one of its descendants, the orphan left behind is enumerated as a child of the reused PID.
///   That orphan was born **before** the reuse — that is, **before** the process that now holds that PID — so this comparison filters all of
///   that path. The case where the root itself dies and the number passes on again is stopped by the first item (root identity check).
///   ★The same time (`==`) passes★ — a parent and child started within one FILETIME tick can really exist, and blocking
///   there would lose a legitimate child. What is blocked is only one **born earlier**.
/// - ★**There are three cases this rule cannot filter — do not read it as 「닫혀 있다」 (closed)**★ (the caller decides whether to accept them):
///   1. **PID reuse between enumeration and start-time read** — if the owner of a number `child_pids` gave dies before the read
///      and the number passes on, the start time read is the new owner's. Enumeration and query are separate syscalls, so there is no means of
///      fixing it atomically (the window is microseconds).
///   2. **The same tick (`==`)** — the one passed above.
///   3. **Explicit parent assignment** — on Windows a process can be created with its parent pinned via `PROC_THREAD_ATTRIBUTE_PARENT_PROCESS`.
///      Someone else's process born that way can be born **after** the root and still carry the root's PID as ppid, so
///      it gets past this comparison.
/// - An entry whose start time cannot be read is **left out, and its subtree is not descended into either** — with half the identity missing the comparison collapses to
///   PID alone, and using that entry as a parent leaves no basis to apply the order rule above, so that whole branch would be unverified.
/// - If `root_start_time` is 0 (unknown), it is an empty list. Same reason.
/// - **best-effort**: if enumeration fails it returns that much less and raises no error — a short list does not confirm 「그 아래에는
///   없다」 (there is nothing under it).
/// - ★There is no depth limit — instead **an identity already seen is not descended into again**★. ppid is not updated by the OS
///   immediately, so shapes that look like cycles can appear, and without a visited mark this function would never finish.
///   ★The mark's key is the **identity**, not the PID★ — if marked by PID alone, a reused PID would be skipped as 「이미 봤다」 (already seen),
///   and a **different** process using the same number would go entirely unseen.
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

/// **Only the rules** of [`subtree`] — the OS is behind `children` (ADR-0012). The reasons for the two rules (discard one born before its
/// parent · do not descend again into an **identity** already seen) are written authoritatively in the doc of [`subtree`] and are not repeated
/// here.
///
/// `children` gives its entries **with those whose identity could not be read already filtered out** — that filtering is outside this rule because
/// an entry without an identity has no order criterion to apply in the first place.
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

/// Forcibly ends `pid` and all its descendants — Windows = `taskkill /PID <pid> /F /T`. It only hands off the ending and does not wait.
/// The exit code of taskkill is not looked at — it answers even for an already-ended process with a non-zero code (128). So
/// `Ok` means not 「끝났다」 (it ended) but 「taskkill 을 돌렸다」 (taskkill was run).
///
/// `Err` = taskkill could not be launched (kind `Other` · the reason is in the message). Outside Windows = `Unsupported` — this
/// means is absent. ★On Windows `Unsupported` does not occur★ — the caller uses that one kind to tell apart "cannot do this on this
/// OS".
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

/// non-windows: process tree enumeration not implemented (the daemon is Windows first).
#[cfg(not(windows))]
pub fn process_parent_table() -> Option<Vec<(u32, u32)>> {
    None
}

#[cfg(not(windows))]
pub fn current_process_start_time() -> Option<u64> {
    None
}

/// non-windows: no means of liveness decision implemented — conservatively treated as alive (start_time ignored).
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
            .args(["/c", "ping -n 3 127.0.0.1 > NUL"]) // A briefly living child
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("cmd.exe spawn");
        let child_pid = child.id();
        let me = std::process::id();

        // There may be a slight delay before ppid is reflected, so poll briefly.
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

    // ── process table · three outcomes of start time (ADR-0257) ──────────────────────────────────────

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
        // Our handle pins the process object, so it can be read even after it has ended.
        let ProcessStart::Known(start) = process_start(pid) else {
            panic!("쥔 자식의 시작시각을 못 읽었다");
        };
        let _ = child.wait();
        drop(child);

        // Other handles (console host etc.) may be released late, so wait until the old identity disappears.
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
            // The number has already passed to someone else — the old identity has disappeared all the same (PID reuse cannot be prevented).
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

    // ── OpenProcess error code → liveness decision (pure, independent of real processes) ────────────────────

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

    // ── identity list under a root (subtree) ─────────────────────────────────────────────────

    const ROOT: ProcessIdentity = ProcessIdentity {
        pid: 4242,
        start_time: 1_000,
    };

    fn id(pid: u32, start_time: u64) -> ProcessIdentity {
        ProcessIdentity { pid, start_time }
    }

    /// Parent PID → the identities under it. If not in the table, no children.
    fn table(rows: &[(u32, Vec<ProcessIdentity>)]) -> impl Fn(u32) -> Vec<ProcessIdentity> + '_ {
        move |pid| {
            rows.iter()
                .find(|(parent, _)| *parent == pid)
                .map(|(_, kids)| kids.clone())
                .unwrap_or_default()
        }
    }

    // ── order rule (G1) ────────────────────────────────────────────────────────────

    /// ★What this entry protects = 「남의 고아가 재사용된 PID 를 타고 나무에 들어오지 않는다」 (someone else's orphan does not enter the tree riding a reused PID)★.
    /// That orphan was up **before** the parent that now holds that PID, so its start time is earlier — that is the only
    /// distinguishing mark.
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

    /// Even what hangs under that other process does not come in — the branch is cut off whole.
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

    /// A child that came up on the same tick is normal — what is blocked is only one born **earlier**.
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

    // ── visited mark (G2) ────────────────────────────────────────────────────────────

    /// ★If the mark is keyed by PID, a reused number is skipped as 「이미 봤다」 (already seen) and a **different** process vanishes entirely★.
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

    /// It finishes even if a stale ppid looks like a cycle — the same identity is not descended into twice.
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

    /// It measures with a real tree (`cmd.exe` → `ping`) that not only the root itself but **descendants enter the list** — if the grandchild
    /// is not caught it is no better than comparing the root alone.
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

        // Wait briefly for the grandchild (`ping`) to come up — cmd comes up first and launches it afterward.
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
