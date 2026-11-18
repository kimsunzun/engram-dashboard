//! PID liveness · process start time (creation time) query and its three outcomes (known, gone, unreadable) · one process table (pid,
//!   ppid) · child PID enumeration · identities of live processes under one root
//! ([`subtree`]) · killing one process tree ([`kill_tree`]) · whether one identity is still running ([`presence`]) and ending it only if
//!   it is ([`terminate_if_same`]).
//!
//! "Is that PID still that process" is decided in several crates (`daemon` portfile stale decision and its own start time in daemon.json,
//!   the shell's `discovery`, `agent`'s codex child identity); all look here instead of keeping a copy
//! — callers: `rg "engram_dashboard_platform::process" crates src-tauri`. ★Do not move it back into the agent runtime★ — consumers outside
//!   agent would depend on the whole runtime (the state ADR-0175 §Context measured).
//!
//! ★Why creation time too★: the OS reuses PIDs; if the daemon dies and another process gets its PID, "PID alive" alone is a false-live. So
//!   the decision is "PID alive AND its current creation time == the recorded value", which
//! tells PID reuse apart directly.

// ADR-0266

/// The result of asking one PID's start time — "could not read" is not lumped into one branch.
/// - `Known` = the creation time of the process with that PID (scale of [`process_creation_time`]). ★Even if it ended, while someone holds
///   the handle (the process object remains) it is `Known`★ — not "alive".
/// - `Gone` = no process object with that number now (only when the open failure is 87).
/// - `Unknown` = existence unknown — could not open (rights or other reason) or the time query failed; PID 0 and an OS without this means
///   land here too.
// ADR-0257
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessStart {
    Known(u64),
    Gone,
    Unknown,
}

/// `Gone` stands only on a certain absence signal — the same single open-failure decision as `pid_alive` (only 87 is absence).
// ADR-0257
#[cfg(windows)]
pub fn process_start(pid: u32) -> ProcessStart {
    use windows::Win32::Foundation::{CloseHandle, FILETIME};
    use windows::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    // The System Idle Process is alive but `OpenProcess` fails with 87 like absence — opening it would read as `Gone`.
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

/// u64 = the high/low 32 bits of GetProcessTimes lpCreationTime (FILETIME — 100 ns intervals since 1601-01-01 UTC) joined. Invariant for
///   one process, different for a reused PID.
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

/// A windows-rs `OpenProcess` failure is a `windows::core::Error` whose `.code().0` is an `HRESULT_FROM_WIN32` HRESULT
///   (INVALID_PARAMETER=0x80070057, ACCESS_DENIED=0x80070005); the low 16 bits of a FACILITY_WIN32 (0x7)
/// HRESULT are the win32 code, recovered with `hr & 0xFFFF` (a non-win32 facility gives a meaningless value, but OpenProcess failures are
///   win32 in practice and the caller's classification absorbs it into the conservative default).
#[cfg(windows)]
fn win32_from_hresult(hr: i32) -> u32 {
    (hr as u32) & 0xFFFF
}

/// ★Why branch per code (false-live bug)★: every OpenProcess failure used to count as live (`Err(_) => true`), so "no such process"
///   (ERROR_INVALID_PARAMETER) read as alive → the app tried to attach to a dead daemon and stuck on
/// 'reconnecting'. Tell absence from lack of rights:
///   - ERROR_INVALID_PARAMETER(87) = no process with that PID → dead(false).
///   - ERROR_ACCESS_DENIED(5) = exists but we lack the right to open → live(true).
///   - any other failure = cause unknown → conservatively live(true) (judging a live daemon dead would throw it away).
/// ★Absence is narrowed to 87 alone (intentional asymmetry)★: MS does not guarantee a dead PID's OpenProcess failure is always 87 (a
///   just-terminated zombie or permission boundary may give ERROR_NOT_FOUND(1168) etc.); guessing more
/// dead codes risks a false-dead. Unlisted absence codes stay false-live — a partial reproduction of the old bug, no new false-dead, and
///   the creation-time comparison in `pid_alive_with_start_time` is a second defense, so no permanent
/// deadlock. Add codes only after measurement (no guesswork — spec poisoning).
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

/// The sibling that ignores creation time — only the conservative fallback when start_time is unknown (0).
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

/// Whether one process identity is still running — the three answers of [`presence`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    /// Running: the creation time matches (or none was given) and the exit code is still `STILL_ACTIVE`.
    Alive,
    /// Not that running process any more: open failed with 87, the creation time differs from a nonzero `start_time` (number reused), or
    ///   the process exited though someone holds its object.
    Gone,
    /// Cannot tell — any other open failure (access denied …) or a failed query. ★Never read it as gone★. PID 0 and an OS without this
    ///   means land here too.
    Unknown,
}

/// Is `pid` still the running process born at `start_time` (scale of [`process_creation_time`]) — decided on **one handle**: open,
///   creation time, exit code. `start_time == 0` (unknown) skips the creation-time comparison,
/// so only 87 or the exit code can say gone. ★An exited process whose object someone holds is gone here★ — [`process_start`] still reads
///   its creation time (`Known`). Known residue: a process that exited with code 259
/// (`STILL_ACTIVE`) reads as running — the OS gives no other signal on this handle.
// TRD S21 trd-3-2-u5 R4
#[cfg(windows)]
pub fn presence(pid: u32, start_time: u64) -> Presence {
    use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    const STILL_ACTIVE: u32 = 259;

    fn decide(handle: HANDLE, start_time: u64) -> Presence {
        if start_time != 0 {
            let mut created = FILETIME::default();
            let mut exited = FILETIME::default();
            let mut kernel = FILETIME::default();
            let mut user = FILETIME::default();
            // SAFETY: a valid handle opened by the caller + four outputs on the stack.
            if unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) }
                .is_err()
            {
                return Presence::Unknown;
            }
            let created =
                (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
            if created != start_time {
                return Presence::Gone;
            }
        }
        let mut code = 0u32;
        // SAFETY: the same handle + an output on the stack.
        if unsafe { GetExitCodeProcess(handle, &mut code) }.is_err() {
            return Presence::Unknown;
        }
        if code == STILL_ACTIVE {
            Presence::Alive
        } else {
            Presence::Gone
        }
    }

    // The System Idle Process fails to open with 87 like an absent one (same guard as `process_start`).
    if pid == 0 {
        return Presence::Unknown;
    }
    // SAFETY: open the PID handle with minimum rights.
    let handle = match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
        Ok(h) => h,
        Err(e) if alive_from_open_error(win32_from_hresult(e.code().0)) => {
            return Presence::Unknown
        }
        Err(_) => return Presence::Gone,
    };
    let answer = decide(handle, start_time);
    // SAFETY: close the valid handle once.
    unsafe {
        let _ = CloseHandle(handle);
    }
    answer
}

/// PID liveness decision (including the start-time comparison). true = alive (the very process we are looking for). Rules (M2 PID-reuse
///   defense) on [`presence`]: pid==0 → dead (the idle process cannot be our daemon);
/// [`Presence::Alive`] → live; [`Presence::Gone`] → dead (an exited process whose object is held is dead); [`Presence::Unknown`] → live
///   only when expected_start==0 (unknown; old daemon.json compatibility —
/// the creation time cannot be compared, so **decide conservatively**, do not label an old file stale), dead with a nonzero expected_start.
#[cfg(windows)]
pub fn pid_alive_with_start_time(pid: u32, expected_start: u64) -> bool {
    if pid == 0 {
        return false;
    }
    match presence(pid, expected_start) {
        Presence::Alive => true,
        Presence::Gone => false,
        Presence::Unknown => expected_start == 0,
    }
}

/// ★Why it is needed★: the AgentInfo/WS protocol does not expose the PTY child's PID (hands/brain separation), but the real-process
///   isolation test (kill the daemon .exe → the PTY child dies with it) must confirm by PID that the
/// launched child died; the only way to learn it from outside is enumerating the OS process tree. best-effort: an empty Vec on
///   snapshot/walk failure. The OS sometimes does not update ppid at once (stale after the parent dies), so
/// it is trusted only as "direct children of a live parent".
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

/// `(pid, ppid)` of all processes currently running — **one** Toolhelp snapshot. `None` = the list could not be read to the end (snapshot
///   creation, first entry, a walk error that is not "end"). ★It never returns a truncated
/// list★ — a missing line would read as "that process does not exist". The ppid caveat is as [`child_pids`].
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

/// Is a single failure of `Process32NextW` the end of the list. windows-rs wraps it as `HRESULT_FROM_WIN32(GetLastError())` and the end is
///   the single `ERROR_NO_MORE_FILES`(18) — anything else means the list was cut off midway.
#[cfg(any(windows, test))]
fn list_ended(hr: i32) -> bool {
    const HRESULT_FROM_ERROR_NO_MORE_FILES: i32 = 0x8007_0012_u32 as i32;
    hr == HRESULT_FROM_ERROR_NO_MORE_FILES
}

// ── identity list under a root ──────────────────────────────────────────────────────────

/// The identity of one process — one process under a root as returned by [`subtree`]. ★The two fields are compared **together**★ — the OS
///   reuses PIDs, so a PID-only match takes someone else's process for that one
/// (ADR-0218 decision 2); this type has no helper taking out only `pid`. [`crate::file_holders::Holder`] has the same shape but means "a
///   process that has a file open" — do not merge them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessIdentity {
    pub pid: u32,
    /// Process creation FILETIME — the same scale as the value [`process_creation_time`] returns.
    pub start_time: u64,
}

/// The identities of `root` and all **live** descendants under it.
/// - ★If `root`'s start time differs from `root_start_time`, the result is an **empty list**★ — that PID is no longer the process the
///   caller knows (died or reused); walking without confirming the root would return someone else's tree.
/// - **Descendants' start times are read now** (no value known beforehand), so the identity returned is always "the process holding that
///   PID at this moment"; a PID reused between enumeration and comparison differs in start time and is filtered.
/// - ★**An entry born before its parent is not a descendant — discard it**★ (the only **semantic** rule). The ppid in [`child_pids`] stays
///   after the parent dies, so if the parent dies and that PID is reused by the root or a descendant,
///   the orphan is enumerated as a child of the reused PID; it was born **before** the process now holding the PID, so this comparison
///   filters it. The root itself dying and the number passing on again is stopped by the first item.
///   ★The same time (`==`) passes★ — a parent and child within one FILETIME tick can really exist; only one **born earlier** is blocked.
/// - ★**Three cases this rule cannot filter — not "closed"**★ (the caller decides whether to accept them): 1. **PID reuse between
///   enumeration and start-time read** (separate syscalls, no atomic fix; microsecond window)
///   2. **the same tick (`==`)** 3. **explicit parent assignment** — `PROC_THREAD_ATTRIBUTE_PARENT_PROCESS` lets someone else's process be
///   born **after** the root yet carry the root's PID as ppid.
/// - An entry whose start time cannot be read is **left out and not descended into** — with half the identity missing the comparison
///   collapses to PID alone and the order rule has no basis, so the branch would be unverified.
/// - `root_start_time == 0` (unknown) → empty list (same reason). **best-effort**: on enumeration failure it returns less and raises no
///   error — a short list does not confirm "nothing under it".
/// - ★No depth limit — **an identity already seen is not descended into again**★: ppid is not updated at once so cycle-looking shapes
///   appear, and without a visited mark it would never finish. ★The mark's key is the **identity**, not the PID★ —
///   keyed by PID, a reused PID would be skipped as "already seen" and a **different** process with that number would go unseen.
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

/// **Only the rules** of [`subtree`] — the OS is behind `children` (ADR-0012); the reasons for the two rules (discard one born before its
///   parent, do not descend again into a seen **identity**) are in the [`subtree`] doc.
/// `children` gives entries **with unreadable identities already filtered out** — an entry without an identity has no order criterion, so
///   that filtering is outside this rule.
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

/// Forcibly ends `pid` and all its descendants — Windows = `taskkill /PID <pid> /F /T`; it only hands off the ending and does not wait.
///   The taskkill exit code is not looked at (an already-ended process answers non-zero, 128), so
/// `Ok` means "taskkill was run", not "it ended". `Err` = taskkill could not be launched (kind `Other`, reason in the message). Outside
///   Windows = `Unsupported` — the caller uses that kind to tell "cannot do this on this OS";
/// ★on Windows `Unsupported` does not occur★.
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
            .map_err(|e| std::io::Error::other(format!("failed to run taskkill: {e}")))
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        Err(std::io::ErrorKind::Unsupported.into())
    }
}

/// Ends that PID if it is still the very process born at `start_time` (creation FILETIME, scale of [`process_creation_time`]) and still
///   running. Start-time check, liveness check and ending go through the **same handle** — while it is
/// held the PID cannot pass to someone else, so a wrong process cannot be killed after checking. Descendants are not touched. `Ok(true)` =
///   ending handed off (no wait) · `Ok(false)` = not that process (absent, start time differs,
/// already ended) · `Err` = could not open, read or end (access denied, ending by itself between check and ending) — not killed. Outside
///   Windows = `Err` of kind `Unsupported` (as [`kill_tree`]).
// ADR-0343
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
        // SAFETY: a valid handle opened by the caller + four outputs on the stack.
        unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) }?;
        let created = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
        if created != start_time {
            return Ok(false);
        }
        let mut code = 0u32;
        // SAFETY: the same handle + stack output.
        unsafe { GetExitCodeProcess(handle, &mut code) }?;
        if code != STILL_ACTIVE {
            return Ok(false);
        }
        // SAFETY: the same handle — it was opened with the terminate right.
        unsafe { TerminateProcess(handle, 1) }?;
        Ok(true)
    }

    let access = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE;
    // SAFETY: opens only the query and terminate rights.
    let handle = match unsafe { OpenProcess(access, false, pid) } {
        Ok(h) => h,
        Err(e) if e.code() == HRESULT::from_win32(ERROR_INVALID_PARAMETER.0) => return Ok(false),
        Err(e) => return Err(std::io::Error::other(format!("open PID {pid}: {e}"))),
    };
    let outcome = decide(handle, start_time)
        .map_err(|e| std::io::Error::other(format!("check · end PID {pid}: {e}")));
    // SAFETY: closes the handle opened above, once.
    unsafe {
        let _ = CloseHandle(handle);
    }
    outcome
}

// ── non-windows stub ─────────────────────────────────────────────────────────────

#[cfg(not(windows))]
pub fn presence(_pid: u32, _start_time: u64) -> Presence {
    Presence::Unknown
}

#[cfg(not(windows))]
pub fn terminate_if_same(_pid: u32, _start_time: u64) -> std::io::Result<bool> {
    Err(std::io::ErrorKind::Unsupported.into())
}

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
        let start = current_process_start_time().expect("own creation time can be queried");
        assert!(start != 0, "creation time must not be 0");
        assert!(pid_alive_with_start_time(pid, start), "self is live");
        assert!(pid_alive(pid), "self is live through OpenProcess too");
    }

    #[cfg(windows)]
    #[test]
    fn current_pid_with_wrong_start_time_is_dead() {
        let pid = std::process::id();
        let real = current_process_start_time().unwrap();
        let wrong = real.wrapping_add(1);
        assert!(
            !pid_alive_with_start_time(pid, wrong),
            "dead on creation time mismatch (PID reuse defense)"
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
            "the spawned child PID ({child_pid}) must appear in child_pids({me})"
        );
    }

    // ── process table · three outcomes of start time (ADR-0257) ──────────────────────────────────────

    #[test]
    fn only_no_more_files_ends_the_list() {
        assert!(list_ended(0x8007_0012_u32 as i32));
        assert!(
            !list_ended(0x8007_0005_u32 as i32),
            "ACCESS_DENIED = truncated"
        );
        assert!(
            !list_ended(0x8007_0018_u32 as i32),
            "BAD_LENGTH = truncated"
        );
        assert!(
            !list_ended(0),
            "a failure with an empty GetLastError = truncated"
        );
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
            let table = process_parent_table().expect("one snapshot");
            if table.contains(&row) {
                found = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let _ = child.kill();
        let _ = child.wait();
        assert!(
            found,
            "the launched child's row {row:?} is not in the table"
        );
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
            panic!("could not read the held child's start time");
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
            other => panic!("ended PID {pid} read as {other:?}"),
        }
    }

    /// TRD S21 trd-3-2-u5 R4: a process that exited while we still hold its object is not alive — its creation time still reads, so a
    ///   creation-time check alone called it live.
    #[cfg(windows)]
    #[test]
    fn an_exited_process_whose_object_is_held_is_gone() {
        let mut child = crate::testing::spawn_gated_cmd("rem", 0);
        let pid = child.id();
        let start = process_creation_time(pid).expect("start time");
        assert_eq!(presence(pid, start), Presence::Alive);
        assert_eq!(presence(pid, 0), Presence::Alive);
        assert!(pid_alive_with_start_time(pid, start));

        crate::testing::open_gate(&mut child);
        let _ = child.wait();
        // `child` still holds the handle, so the process object (and its creation time) stays readable.
        assert_eq!(process_start(pid), ProcessStart::Known(start));

        assert_eq!(presence(pid, start), Presence::Gone);
        assert_eq!(
            presence(pid, 0),
            Presence::Gone,
            "the exit code decides without a start time"
        );
        assert!(!pid_alive_with_start_time(pid, start));
        assert!(!pid_alive_with_start_time(pid, 0));
        drop(child);
    }

    #[cfg(windows)]
    #[test]
    fn presence_of_a_reused_number_is_gone_and_pid_zero_is_unknown() {
        let me = std::process::id();
        let start = current_process_start_time().unwrap();
        assert_eq!(presence(me, start), Presence::Alive);
        assert_eq!(
            presence(me, start.wrapping_add(1)),
            Presence::Gone,
            "a nonzero start time that differs = another process on that number"
        );
        assert_eq!(
            presence(0, start),
            Presence::Unknown,
            "PID 0 is never read as gone"
        );
    }

    /// A different start time does not kill; only the same identity kills — after the ending even the same identity is "not that process".
    #[cfg(windows)]
    #[test]
    fn terminate_if_same_ends_only_the_matching_identity() {
        let mut child = crate::testing::spawn_gated_cmd("rem", 0);
        let pid = child.id();
        let start = process_creation_time(pid).expect("start time");

        let other = terminate_if_same(pid, start + 1);
        let alive_after_other = pid_alive(pid);
        let same = terminate_if_same(pid, start);
        let _ = child.kill();
        let _ = child.wait();
        let after_exit = terminate_if_same(pid, start);

        assert!(matches!(other, Ok(false)), "{other:?}");
        assert!(
            alive_after_other,
            "killed it with an identity whose start time differs"
        );
        assert!(matches!(same, Ok(true)), "{same:?}");
        assert!(matches!(after_exit, Ok(false)), "{after_exit:?}");
    }

    #[cfg(windows)]
    #[test]
    fn unknown_start_time_falls_back_to_pid_liveness() {
        let pid = std::process::id();
        assert!(
            pid_alive_with_start_time(pid, 0),
            "if unknown, conservatively decided by PID liveness"
        );
    }

    // ── OpenProcess error code → liveness decision (pure, independent of real processes) ────────────────────

    #[cfg(windows)]
    #[test]
    fn alive_from_open_error_classifies_codes() {
        assert!(!alive_from_open_error(87), "absent → dead");
        assert!(
            alive_from_open_error(5),
            "insufficient permission but exists → live"
        );
        assert!(alive_from_open_error(0), "unknown → conservatively live");
        assert!(alive_from_open_error(1234), "unknown → conservatively live");
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

    /// ★Protects "someone else's orphan does not enter the tree riding a reused PID"★: that orphan was up **before** the parent now
    ///   holding the PID, so its earlier start time is the only distinguishing mark.
    #[test]
    fn a_child_that_predates_its_parent_is_not_ours() {
        let stranger = id(777, ROOT.start_time - 1);
        let got = walk(ROOT, &table(&[(ROOT.pid, vec![stranger])]));
        assert_eq!(
            got,
            vec![ROOT],
            "an item born before its parent came in: {got:?}"
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
        assert_eq!(
            got,
            vec![ROOT],
            "something below a cut branch came in: {got:?}"
        );
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

    /// ★Keyed by PID, a reused number is skipped as "already seen" and a **different** process vanishes entirely★.
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
            "another process with the same number was swallowed by the mark: {got:?}"
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
            "if the start time is unknown, what remains is a PID-only match"
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_wrong_start_time_disowns_the_root() {
        let me = std::process::id();
        let start = process_creation_time(me).expect("own creation time can be queried");
        assert!(
            subtree(me, start.wrapping_add(1)).is_empty(),
            "took a PID with a mismatched start time as that root"
        );
    }

    /// Measures with a real tree (`cmd.exe` → `ping`) that **descendants enter the list**, not just the root — a grandchild not caught is
    ///   no better than comparing the root alone.
    #[cfg(windows)]
    #[test]
    fn the_subtree_reaches_past_the_root() {
        let mut child = std::process::Command::new("cmd.exe")
            .args(["/c", "ping", "-n", "4", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("launch cmd.exe");
        let root = child.id();
        let start = process_creation_time(root).expect("child creation time can be queried");

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
            "the root itself is not in the list: {found:?}"
        );
        assert!(
            found.len() > 1,
            "not a single descendant was caught — as if it returned only the root PID: {found:?}"
        );
        assert!(
            found.iter().all(|p| p.start_time != 0),
            "an item with half of its identity empty is mixed in: {found:?}"
        );
    }
}
