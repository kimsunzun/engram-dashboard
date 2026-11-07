//! Windows Job Object wrapper + a tool that wakes a process launched suspended ([`resume_suspended_process`]) — the Windows
//! branch of the outer module (`group`).
//!
//! It ties child processes into a Job, so that when we kill explicitly (TerminateJobObject)
//! or the host process crashes (KILL_ON_JOB_CLOSE), grandchild processes are cleaned up together.
//! The parts that handle one member at a time (list, pinning and facts, ending) and the Job join-notification port are here too — it does not
//! know what to end; it only checks that the number given by the chooser is a member of this Job and pins it (ADR-0262). The exit code to give when ending is also
//! supplied by the caller (ADR-0275 decision 12).
//!
//! ★A Job is inherited only by processes created **after** they are inside it★ — if an already-running process is put in with [`JobObjectHandle::assign`],
//!   children it launched before being put in stay outside the Job and escape a tree kill. The tree root (used by the agent usage query)
//!   is launched suspended (`CREATE_SUSPENDED`) via `spawn::prepare_tree_root`, and `spawn::TreeRoot` puts it in and then wakes it,
//!   closing that gap. ★The agent's agent paths (`transport::pty`, `transport::stdio`, the codex path) put the process in after launching, so they
//!   still carry that gap★.
//!
//! The call order and flags are the same as verified by measurement on Windows in the Phase 0 spike (agent `examples/spike.rs`).
//! ★Nothing is used except the `windows` crate and the standard library★ — the result, fact and notification types are also kept here and the outer layer moves them
//! into its own public types (ADR-0262 「[구현] 고른 것」 (Implementation: what was chosen) — user decision 2026-09-29). Only the tests borrow `process` and
//! `testing` from the same crate.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use windows::core::{HRESULT, PWSTR};
use windows::Wdk::System::Threading::{
    NtQueryInformationProcess, ProcessBasicInformation, ProcessCommandLineInformation,
};
use windows::Win32::Foundation::{
    CloseHandle, BOOL, ERROR_INVALID_PARAMETER, ERROR_MORE_DATA, FILETIME, HANDLE,
    INVALID_HANDLE_VALUE, UNICODE_STRING, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob,
    JobObjectAssociateCompletionPortInformation, JobObjectBasicAccountingInformation,
    JobObjectBasicProcessIdList, JobObjectExtendedLimitInformation, QueryInformationJobObject,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_ASSOCIATE_COMPLETION_PORT,
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_BASIC_PROCESS_ID_LIST,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows::Win32::System::Threading::{
    GetProcessIdOfThread, GetProcessTimes, OpenProcess, OpenThread, QueryFullProcessImageNameW,
    ResumeThread, TerminateProcess, WaitForSingleObject, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
    THREAD_QUERY_LIMITED_INFORMATION, THREAD_SUSPEND_RESUME,
};
use windows::Win32::System::IO::{CreateIoCompletionPort, GetQueuedCompletionStatus, OVERLAPPED};

/// Number of slots for the first list query in production. It grows if too small.
const MEMBER_LIST_INITIAL: usize = 64;
/// List re-ask limit — if members keep growing while asking and a complete list cannot be obtained, it stops here (closed failure).
const MEMBER_LIST_ATTEMPTS: usize = 4;

/// The Job notification number carried in the byte-count field of a completion port packet — a member newly joined. It is kept here so as not to
/// turn on `Win32_System_SystemServices` for this one constant.
const JOB_OBJECT_MSG_NEW_PROCESS: u32 = 6;
/// The key given when attaching the Job to the port — only packets that come with this key are read as Job notifications.
const BIRTH_PORT_KEY: usize = 1;
/// Executable path buffer (UTF-16 slots) — the maximum length of a Win32 path.
const IMAGE_PATH_CHARS: usize = 32_768;
/// Command-line buffer — a size where the longest command line (32,767 chars) fits after the header (`UNICODE_STRING`). If too small the command line is an empty field.
const COMMAND_LINE_BYTES: usize = core::mem::size_of::<UNICODE_STRING>() + 65_536;

/// Result layout of `NtQueryInformationProcess(ProcessBasicInformation)` — kept here so as not to turn on `Win32_System_Kernel`.
/// Pointer fields are received as `usize` of the same size and alignment.
#[allow(non_camel_case_types)]
#[repr(C)]
#[derive(Default)]
struct PROCESS_BASIC_INFORMATION {
    _exit_status: i32,
    _peb_base_address: usize,
    _affinity_mask: usize,
    _base_priority: i32,
    _unique_process_id: usize,
    inherited_from_unique_process_id: usize,
}

/// Facts read once through the handle pinned by [`JobObjectHandle::pin_member`]. The meaning of the fields and the value of unread fields are the same as the outer public facts
/// type — this file keeps its own shape so as not to call the outer layer (module header).
#[derive(Debug, Clone, Default)]
pub(crate) struct MemberFacts {
    pub ppid: u32,
    pub create: u64,
    pub image: String,
    pub cmdline: String,
}

/// The answer of [`BirthPort::next`] — the meaning is the same as the outer public notification type (the reason to keep it separate is the same as for [`MemberFacts`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PortOutcome {
    Joined(u32),
    Other,
    Timeout,
}

/// A process confirmed as a member of our Job and pinned — while it is held that PID is not reused (the open handle pins
/// the process object). It can be held even after it ends. drop = CloseHandle.
#[derive(Debug)]
pub(crate) struct PinnedMember {
    handle: HANDLE,
    facts: MemberFacts,
}

// SAFETY: a process handle may be used from any thread. The methods only do OS queries and terminate on the handle, and closing is the single drop
//   that this value solely owns.
unsafe impl Send for PinnedMember {}
unsafe impl Sync for PinnedMember {}

/// The Job join-notification port — [`JobObjectHandle::watch_births`] creates it. Several threads share it through `Arc`, and the last
/// drop closes the port handle.
///
/// Notifications are only notices and delivery is not guaranteed (MS Learn) — a missed join cannot be known here.
///
/// ★Do not let go of the last `Arc` while the Job is attached — detach first ([`JobObjectHandle::unwatch_births`]), then let go★: while attached
/// the Job holds the port as its own kernel reference, so the port survives even if the handle closes, and notifications keep piling up on a port nobody reads.
/// That kernel reference is not written in MS Learn and was not measured here — even if wrong, detaching first loses nothing.
#[derive(Debug)]
pub(crate) struct BirthPort {
    handle: HANDLE,
}

// SAFETY: a completion port handle is a kernel object that several threads may take from at the same time. Closing is the single drop of the last `Arc`.
unsafe impl Send for BirthPort {}
unsafe impl Sync for BirthPort {}

/// The answer of [`PinnedMember::classify`]. `Gone` = it ended by itself after being pinned and before the terminate.
// ADR-0262
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MemberOutcome {
    Terminated,
    Gone,
}

pub(crate) struct JobObjectHandle {
    handle: HANDLE,
}

// SAFETY: HANDLE is a raw pointer wrapper, so it is not automatically Send/Sync.
// We alone manage ownership of the Job handle through create, terminate and CloseHandle,
// and there is no concurrent mutation (after creation assign/terminate are called read-only), so moving and sharing across threads is allowed.
unsafe impl Send for JobObjectHandle {}
unsafe impl Sync for JobObjectHandle {}

impl JobObjectHandle {
    pub(crate) fn new() -> io::Result<Self> {
        // SAFETY: CreateJobObjectW — all arguments are None (an anonymous Job with no security attributes and no name).
        let handle = unsafe { CreateJobObjectW(None, None) }.map_err(win_err)?;

        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

        // SAFETY: SetInformationJobObject — passes the valid Job handle created above
        // the pointer to the info struct on the stack and its exact size. The class and the struct type match (Extended).
        let result = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                core::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if let Err(e) = result {
            // SAFETY: closes the valid handle just created, exactly once.
            unsafe {
                let _ = CloseHandle(handle);
            }
            return Err(win_err(e));
        }

        Ok(Self { handle })
    }

    /// ★If you put it in after launching, children it launched before being put in remain outside the Job★ (the gap of the three agent paths) — the precedent that closes it =
    /// launch suspended via `spawn::prepare_tree_root`, and `spawn::TreeRoot` puts it in and then wakes it with [`resume_suspended_process`].
    pub(crate) fn assign(&self, process_id: u32) -> io::Result<()> {
        // SAFETY: OpenProcess — opens only the minimum rights that AssignProcessToJobObject requires
        // (SET_QUOTA|TERMINATE).
        let process =
            unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, false, process_id) }
                .map_err(win_err)?;

        // SAFETY: AssignProcessToJobObject — a valid Job handle and the process handle just opened.
        let result = unsafe { AssignProcessToJobObject(self.handle, process) };

        // Job membership persists regardless of the process handle, so it is safe to close it here.
        // SAFETY: closes the valid handle returned by OpenProcess, exactly once.
        unsafe {
            let _ = CloseHandle(process);
        }

        result.map_err(win_err)
    }

    pub(crate) fn terminate(&self, exit_code: u32) -> io::Result<()> {
        // SAFETY: TerminateJobObject — a valid Job handle. Forcibly ends all processes in the Job
        // with the given exit code.
        unsafe { TerminateJobObject(self.handle, exit_code) }.map_err(win_err)
    }

    /// Number of processes in the Job that have not yet ended. `terminate` only starts the ending and returns, so a side that waits for 「트리가 다
    /// 끝났다」 (the tree has all ended) checks with this.
    pub(crate) fn active_processes(&self) -> io::Result<u32> {
        let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        // SAFETY: QueryInformationJobObject — passes a valid Job handle, a stack struct pointer matching the class (BasicAccounting)
        // and the exact size. The returned length is not taken (None).
        unsafe {
            QueryInformationJobObject(
                self.handle,
                JobObjectBasicAccountingInformation,
                &mut info as *mut _ as *mut core::ffi::c_void,
                core::mem::size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                None,
            )
        }
        .map_err(win_err)?;
        Ok(info.ActiveProcesses)
    }
    /// All PIDs of processes now in this Job (including members of nested Jobs; the order has no meaning).
    ///
    /// ★`Ok` only when complete★ — if the list count does not equal the Job's member count within the re-ask limit it is `Err`. A missing member
    /// would read to the caller as 「우리 것이 아니다」 (not ours).
    // ADR-0262
    pub(crate) fn member_pids(&self) -> io::Result<Vec<u32>> {
        self.member_pids_with_capacity(MEMBER_LIST_INITIAL)
    }

    /// Same rule as [`Self::member_pids`] · only the first slot count is injected — tests measure the grow and re-ask path with few processes.
    pub(crate) fn member_pids_with_capacity(&self, initial: usize) -> io::Result<Vec<u32>> {
        const HEAD_WORDS: usize =
            core::mem::offset_of!(JOBOBJECT_BASIC_PROCESS_ID_LIST, ProcessIdList)
                / core::mem::size_of::<usize>();
        const _: () = assert!(
            core::mem::offset_of!(JOBOBJECT_BASIC_PROCESS_ID_LIST, ProcessIdList)
                % core::mem::size_of::<usize>()
                == 0
        );

        let mut capacity = initial.max(1);
        let mut last = (0u32, 0u32, capacity, false);
        for _ in 0..MEMBER_LIST_ATTEMPTS {
            // Taken as usize slots to keep the struct alignment — a ULONG_PTR entry follows the two u32 header fields.
            let mut buf = vec![0usize; HEAD_WORDS + capacity];
            let bytes = u32::try_from(core::mem::size_of_val(buf.as_slice())).map_err(|_| {
                io::Error::other(format!("Job 멤버 명단 버퍼가 너무 크다({capacity} 칸)"))
            })?;
            // SAFETY: a valid Job handle + a writable `bytes`-byte buffer (usize-aligned, header + at least 1 entry slot).
            let asked = unsafe {
                QueryInformationJobObject(
                    self.handle,
                    JobObjectBasicProcessIdList,
                    buf.as_mut_ptr().cast(),
                    bytes,
                    None,
                )
            };
            let replied_whole = match asked {
                Ok(()) => true,
                Err(e) if e.code() == HRESULT::from_win32(ERROR_MORE_DATA.0) => false,
                Err(e) => return Err(win_err(e)),
            };
            let head = buf.as_ptr().cast::<JOBOBJECT_BASIC_PROCESS_ID_LIST>();
            // SAFETY: buf is at least one struct in size and usize-aligned. Fields are read by value, without references.
            let (assigned, listed) = unsafe {
                (
                    core::ptr::addr_of!((*head).NumberOfAssignedProcesses).read(),
                    core::ptr::addr_of!((*head).NumberOfProcessIdsInList).read(),
                )
            };
            last = (assigned, listed, capacity, !replied_whole);
            if replied_whole && listed == assigned {
                let entries = buf
                    .get(HEAD_WORDS..HEAD_WORDS + listed as usize)
                    .ok_or_else(|| {
                        io::Error::other(format!(
                            "Job 멤버 명단이 {listed} 개라는데 버퍼는 {capacity} 칸이다"
                        ))
                    })?;
                return entries
                    .iter()
                    .map(|&pid| {
                        u32::try_from(pid).map_err(|_| {
                            io::Error::other(format!("Job 멤버 PID {pid:#x} 가 u32 를 넘는다"))
                        })
                    })
                    .collect();
            }
            let wanted = (assigned as usize).saturating_mul(2).saturating_add(16);
            // Even for an answer that did not fill the header (member count 0), the slots must grow for re-asking to mean anything.
            capacity = if wanted > capacity {
                wanted
            } else {
                capacity.saturating_mul(2)
            };
        }
        let (assigned, listed, asked, more_data) = last;
        Err(io::Error::other(format!(
            "Job 멤버 명단이 {MEMBER_LIST_ATTEMPTS} 번 안에 완전해지지 않았다 — 마지막 답: 멤버 {assigned} · \
             명단 {listed} · 물은 칸 {asked} · 잘린 답 {more_data}"
        )))
    }

    /// Opens the process with that number, pins it **only when it is a member of this Job**, and reads the facts once. It does not check whether it is alive
    /// ([`PinnedMember::exited`]).
    ///
    /// `kill` = also opens the terminate right — for one pinned with `false`, [`PinnedMember::terminate_raw`] always fails.
    /// `Ok(None)` = no process with that number · it is outside this Job. `Err` = failure to open (other than absence — access denied etc.) ·
    /// failure of the membership query.
    pub(crate) fn pin_member(&self, pid: u32, kill: bool) -> io::Result<Option<PinnedMember>> {
        let mut access = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE;
        if kill {
            access |= PROCESS_TERMINATE;
        }
        // SAFETY: opens only the query, wait (and terminate) rights.
        let handle = match unsafe { OpenProcess(access, false, pid) } {
            Ok(h) => h,
            Err(e) if e.code() == HRESULT::from_win32(ERROR_INVALID_PARAMETER.0) => {
                return Ok(None)
            }
            Err(e) => return Err(win_err(e)),
        };
        let mut member = PinnedMember {
            handle,
            facts: MemberFacts::default(),
        };

        let mut in_job = BOOL::default();
        // SAFETY: two valid handles + stack output. ★The Job argument is always our handle★ — if it were NULL it would turn into 「아무 Job 에나
        //   들었나」 (is it in any Job at all) and see someone else's Job member as ours.
        unsafe { IsProcessInJob(member.handle, self.handle, &mut in_job) }.map_err(win_err)?;
        if !in_job.as_bool() {
            return Ok(None);
        }
        member.facts = read_facts(member.handle);
        Ok(Some(member))
    }

    /// Creates the join-notification port and passes it to `start`, and attaches this Job to that port **only when `start` is `Ok`** — by the time notifications can
    /// pile up, the side that takes them out already holds the port. `start`'s `Err` is returned as is (nothing was attached).
    ///
    /// Members already inside are announced again as `Joined` when attaching · notifications for a member whose state changes during attaching may be missed
    /// (MS Learn). ★Call it only once per Job★ — a second call is not blocked here (that is the caller's job).
    pub(crate) fn watch_births(
        &self,
        start: impl FnOnce(Arc<BirthPort>) -> io::Result<()>,
    ) -> io::Result<()> {
        // SAFETY: creates only a new port, without a file (INVALID_HANDLE_VALUE · no existing port · concurrency 1).
        let handle =
            unsafe { CreateIoCompletionPort(INVALID_HANDLE_VALUE, HANDLE::default(), 0, 1) }
                .map_err(win_err)?;
        let port = Arc::new(BirthPort { handle });
        start(Arc::clone(&port))?;
        let attached = self.associate_port(port.handle);
        // Let go after attaching ends — if the `start` side lets go first and the handle closes, that value is reused and someone else's handle could be attached.
        drop(port);
        attached
    }

    /// Detaches this Job's port connection (NULL in the port slot — the detach MS Learn describes). Members born after detaching are not announced.
    pub(crate) fn unwatch_births(&self) -> io::Result<()> {
        self.associate_port(HANDLE::default())
    }

    fn associate_port(&self, port: HANDLE) -> io::Result<()> {
        let info = JOBOBJECT_ASSOCIATE_COMPLETION_PORT {
            CompletionKey: core::ptr::without_provenance_mut(BIRTH_PORT_KEY),
            CompletionPort: port,
        };
        // SAFETY: a valid Job handle + a stack struct pointer and its exact size. The class and the struct type match.
        unsafe {
            SetInformationJobObject(
                self.handle,
                JobObjectAssociateCompletionPortInformation,
                &info as *const _ as *const core::ffi::c_void,
                core::mem::size_of::<JOBOBJECT_ASSOCIATE_COMPLETION_PORT>() as u32,
            )
        }
        .map_err(win_err)
    }
}

impl PinnedMember {
    /// Has it ended now — it does not wait. `Err` = failure of the wait query itself.
    pub(crate) fn exited(&self) -> io::Result<bool> {
        signaled_within(self.handle, 0)
    }

    /// Waits up to `limit` for it to end — `true` if it ended. `Err` = failure of the wait query itself.
    pub(crate) fn wait_exit(&self, limit: Duration) -> io::Result<bool> {
        signaled_within(self.handle, finite_millis(limit))
    }

    pub(crate) fn facts(&self) -> &MemberFacts {
        &self.facts
    }

    /// Makes **only one `TerminateProcess` call**, giving `exit_code`, on that pinned handle. Asynchronous — it does not wait for the target to
    /// end. It returns the raw result only; sorting is done by [`Self::classify`] (narrowed so that it can be called inside a
    /// lock). One pinned with `kill = false` is always `Err`.
    pub(crate) fn terminate_raw(&self, exit_code: u32) -> io::Result<()> {
        terminate_handle(self.handle, exit_code)
    }

    /// Sorts the raw result of [`Self::terminate_raw`]. Even on failure, if it ended by itself in the meantime it is `Gone`. The handle is not
    /// closed (drop closes it).
    pub(crate) fn classify(&self, raw: io::Result<()>) -> io::Result<MemberOutcome> {
        classify_terminate(self.handle, raw)
    }
}

impl BirthPort {
    /// Waits up to `wait` for one notification and takes it out. `Err` = failure of taking it out itself (port closed etc.).
    pub(crate) fn next(&self, wait: Duration) -> io::Result<PortOutcome> {
        let mut message = 0u32;
        let mut key = 0usize;
        let mut overlapped: *mut OVERLAPPED = core::ptr::null_mut();
        // SAFETY: a valid port handle + three stack outputs · finite wait.
        let dequeued = unsafe {
            GetQueuedCompletionStatus(
                self.handle,
                &mut message,
                &mut key,
                &mut overlapped,
                finite_millis(wait),
            )
        };
        match dequeued {
            Ok(()) if key == BIRTH_PORT_KEY && message == JOB_OBJECT_MSG_NEW_PROCESS => {
                // A Job notification carries a PID, not a pointer, in the OVERLAPPED field — it is not dereferenced.
                let pid = overlapped as usize;
                u32::try_from(pid).map(PortOutcome::Joined).map_err(|_| {
                    io::Error::other(format!("Job 가입 알림의 PID {pid:#x} 가 u32 를 넘는다"))
                })
            }
            Ok(()) => Ok(PortOutcome::Other),
            Err(e) if e.code() == HRESULT::from_win32(WAIT_TIMEOUT.0) => Ok(PortOutcome::Timeout),
            Err(e) => Err(win_err(e)),
        }
    }
}

impl Drop for PinnedMember {
    fn drop(&mut self) {
        // SAFETY: closes the valid handle that pin_member opened, exactly once.
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

impl Drop for BirthPort {
    fn drop(&mut self) {
        // SAFETY: the last `Arc` closes the valid port handle that watch_births created, exactly once.
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

/// One `TerminateProcess` call. ★Even failure is built as an error that does not use the heap★ — since it is called inside a lock,
/// it does not use `win_err` (`io::Error::other` = a box allocation). The `ok()` of windows 0.58 only reads `GetLastError`, so
/// the last error is still there.
// ADR-0262
fn terminate_handle(handle: HANDLE, exit_code: u32) -> io::Result<()> {
    // SAFETY: a valid process handle held by the caller — if there is no terminate right the OS refuses.
    unsafe { TerminateProcess(handle, exit_code) }.map_err(|_| io::Error::last_os_error())
}

/// Sorting the raw terminate result — even on failure, if it ended by itself in the meantime it is `Gone`.
// ADR-0262
fn classify_terminate(handle: HANDLE, raw: io::Result<()>) -> io::Result<MemberOutcome> {
    match raw {
        Ok(()) => Ok(MemberOutcome::Terminated),
        // SAFETY: a valid handle held by the caller · a query that does not wait.
        Err(_) if unsafe { WaitForSingleObject(handle, 0) } == WAIT_OBJECT_0 => {
            Ok(MemberOutcome::Gone)
        }
        Err(e) => Err(e),
    }
}

/// Wait in ms — `u32::MAX` is INFINITE, so it is clamped below that.
fn finite_millis(wait: Duration) -> u32 {
    u32::try_from(wait.as_millis())
        .unwrap_or(u32::MAX)
        .min(u32::MAX - 1)
}

/// Did the handle get signaled within `millis` (= has it ended). Liveness is not measured by the exit code (STILL_ACTIVE = 259) — a process that
/// ended with 259 would look alive.
fn signaled_within(handle: HANDLE, millis: u32) -> io::Result<bool> {
    // SAFETY: a valid handle held by the caller · finite wait.
    match unsafe { WaitForSingleObject(handle, millis) } {
        WAIT_OBJECT_0 => Ok(true),
        WAIT_TIMEOUT => Ok(false),
        WAIT_FAILED => Err(io::Error::last_os_error()),
        other => Err(io::Error::other(format!(
            "프로세스 대기가 뜻밖의 값 {:#x} 를 돌려줬다",
            other.0
        ))),
    }
}

fn read_facts(handle: HANDLE) -> MemberFacts {
    MemberFacts {
        ppid: parent_pid(handle).unwrap_or(0),
        create: creation_time(handle).unwrap_or(0),
        image: image_path(handle).unwrap_or_default(),
        cmdline: command_line(handle).unwrap_or_default(),
    }
}

fn parent_pid(handle: HANDLE) -> Option<u32> {
    let mut info = PROCESS_BASIC_INFORMATION::default();
    let size = core::mem::size_of::<PROCESS_BASIC_INFORMATION>() as u32;
    let mut written = 0u32;
    // SAFETY: a valid handle + a stack struct set up per that class's layout and its exact size + stack output.
    let status = unsafe {
        NtQueryInformationProcess(
            handle,
            ProcessBasicInformation,
            (&mut info as *mut PROCESS_BASIC_INFORMATION).cast(),
            size,
            &mut written,
        )
    };
    // The layout is trusted only if the returned size also matches.
    if status.is_err() || written != size {
        return None;
    }
    u32::try_from(info.inherited_from_unique_process_id).ok()
}

fn creation_time(handle: HANDLE) -> Option<u64> {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: a valid handle + four FILETIME output pointers on the stack.
    unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) }.ok()?;
    Some(((creation.dwHighDateTime as u64) << 32) | (creation.dwLowDateTime as u64))
}

pub(crate) fn image_path(handle: HANDLE) -> Option<String> {
    let mut buf = vec![0u16; IMAGE_PATH_CHARS];
    let mut len = IMAGE_PATH_CHARS as u32;
    // SAFETY: a valid handle + a writable buffer whose length is given + stack in/out length.
    unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
    }
    .ok()?;
    buf.get(..len as usize).map(String::from_utf16_lossy)
}

/// That it can be read with the `QUERY_LIMITED` right is a measurement, not documentation (TRD t40 §8 ㉒ⓘ) — if refused it becomes an empty field.
fn command_line(handle: HANDLE) -> Option<String> {
    const HEAD: usize = core::mem::size_of::<UNICODE_STRING>();
    // Taken as u64 slots to match the alignment of the pointer fields in the header.
    let mut buf = vec![0u64; COMMAND_LINE_BYTES.div_ceil(8)];
    let bytes = buf.len() * 8;
    let mut written = 0u32;
    // SAFETY: a valid handle + a writable `bytes`-byte buffer (8-aligned) + stack output.
    let status = unsafe {
        NtQueryInformationProcess(
            handle,
            ProcessCommandLineInformation,
            buf.as_mut_ptr().cast(),
            u32::try_from(bytes).ok()?,
            &mut written,
        )
    };
    if status.is_err() {
        return None;
    }
    // SAFETY: the buffer is at least the header size and 8-aligned. `UNICODE_STRING` is Copy, so it is read by value.
    let head = unsafe { buf.as_ptr().cast::<UNICODE_STRING>().read() };
    let units = usize::from(head.Length) / 2;
    if units == 0 {
        return Some(String::new());
    }
    // The kernel fills the characters after the header inside our buffer and Buffer points at that spot — that address is not trusted as is; it is
    //   converted to an offset inside our buffer, the range is checked, and then it is read from our buffer.
    let offset = (head.Buffer.0 as usize).checked_sub(buf.as_ptr() as usize)?;
    if offset < HEAD || !offset.is_multiple_of(2) || offset.checked_add(units * 2)? > bytes {
        return None;
    }
    // SAFETY: [offset, offset + units * 2) is a range inside the buffer checked above and is 2-aligned (the buffer is 8-aligned).
    let text = unsafe {
        core::slice::from_raw_parts(buf.as_ptr().cast::<u8>().add(offset).cast::<u16>(), units)
    };
    Some(String::from_utf16_lossy(text))
}

impl Drop for JobObjectHandle {
    fn drop(&mut self) {
        // When the handle closes, thanks to KILL_ON_JOB_CLOSE the OS also cleans up remaining processes.
        // SAFETY: closes the valid Job handle created in new() exactly once on Drop.
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

/// Wakes the suspended thread of process `process_id`, launched with `CREATE_SUSPENDED` (a just-launched process has one first thread).
///
/// std's `Child` does not hand out the first thread handle (it closes it right after startup) — so the process's thread is
/// looked up in a thread snapshot. Threads that vanished after the snapshot, and someone else's thread that took over their number, are skipped (not woken).
///
/// ★It is an error if not even one suspended thread was woken★ — calling it on a process that is not suspended fails. So if the launching side
///   loses launch-suspended (the child starts running before it is put in the Job), startup fails instead of silently reviving the gap.
///   If that process's own thread cannot be woken (`ResumeThread` failed), that is the error.
pub fn resume_suspended_process(process_id: u32) -> io::Result<()> {
    // SAFETY: a thread snapshot is system-wide, so the process argument is ignored (0).
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }.map_err(win_err)?;
    let mut entry = THREADENTRY32 {
        dwSize: core::mem::size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };
    let mut resumed = 0usize;
    let mut seen = 0usize;
    let mut failure = None;
    // SAFETY: a valid snapshot handle + a stack entry with dwSize filled in.
    let mut more = unsafe { Thread32First(snapshot, &mut entry) }.is_ok();
    while more {
        if entry.th32OwnerProcessID == process_id {
            seen += 1;
            match resume_own_thread(entry.th32ThreadID, process_id) {
                Ok(Resumed::WasSuspended) => resumed += 1,
                Ok(Resumed::NotSuspended | Resumed::NotOurs) => {}
                Err(e) => {
                    failure = Some(e);
                    break;
                }
            }
        }
        // SAFETY: the same valid snapshot handle + entry.
        more = unsafe { Thread32Next(snapshot, &mut entry) }.is_ok();
    }
    // SAFETY: closes the snapshot handle exactly once.
    unsafe {
        let _ = CloseHandle(snapshot);
    }
    match (failure, resumed) {
        (Some(e), _) => Err(e),
        (None, 0) => Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("그 프로세스에 멈춘 스레드가 없다(스냅숏에서 본 스레드 {seen}개)"),
        )),
        (None, _) => Ok(()),
    }
}

enum Resumed {
    /// Lowered the suspend count of the suspended thread by one.
    WasSuspended,
    /// It was a thread that was already running — `ResumeThread` changed nothing.
    NotSuspended,
    /// It vanished after the snapshot or its number passed to someone else's process — it was not touched.
    NotOurs,
}

fn resume_own_thread(thread_id: u32, process_id: u32) -> io::Result<Resumed> {
    // SAFETY: OpenThread — opens only the rights that ResumeThread and GetProcessIdOfThread require.
    let opened = unsafe {
        OpenThread(
            THREAD_SUSPEND_RESUME | THREAD_QUERY_LIMITED_INFORMATION,
            false,
            thread_id,
        )
    };
    // A thread that does not open = it ended after the snapshot (other causes fall in here too — if nothing was woken, the caller fails).
    let Ok(thread) = opened else {
        return Ok(Resumed::NotOurs);
    };
    // SAFETY: the valid thread handle just opened. On failure it is 0 — 0 is the number of the System Idle process, so it cannot be the number of the
    // child we launched, and so the comparison below sorts it as 「남의 것」 (someone else's).
    let owner = unsafe { GetProcessIdOfThread(thread) };
    let result = if owner != process_id {
        Ok(Resumed::NotOurs)
    } else {
        // SAFETY: the same valid handle. Return value = the suspend count before the call, failure = `u32::MAX` (the documented `(DWORD)-1`).
        match unsafe { ResumeThread(thread) } {
            // The last error is read before CloseHandle overwrites it.
            u32::MAX => Err(io::Error::last_os_error()),
            0 => Ok(Resumed::NotSuspended),
            _ => Ok(Resumed::WasSuspended),
        }
    };
    // SAFETY: closes the valid handle that OpenThread returned, exactly once.
    unsafe {
        let _ = CloseHandle(thread);
    }
    result
}

/// Passing an HRESULT to from_raw_os_error wraps it as 0x8007xxxx and garbles the message, so
/// io::Error::other is used to preserve the original error (message included).
fn win_err(e: windows::core::Error) -> io::Error {
    io::Error::other(e)
}

// Real-process tests — at most four per test (console host included): do not launch them in a burst (local parallel limit `--test-threads=4` — `.claude/skill-bindings/qa.md` 「모듈별 명령」).
#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::Instant;

    use windows::Win32::System::Threading::{CREATE_NO_WINDOW, DETACHED_PROCESS};

    use crate::process::{process_start, ProcessStart};
    use crate::testing::{is_ping, open_gate, spawn_gated_cmd, wait_until};

    use super::*;

    /// The terminate exit code chosen by the test — it only needs to differ from terminating the whole Job (1).
    const EXIT_CODE: u32 = 0x7E57;

    /// Takes notifications out until one that matches `want` arrives (10 s) — earlier notifications are discarded.
    fn next_matching(
        port: &BirthPort,
        what: &str,
        mut want: impl FnMut(PortOutcome) -> bool,
    ) -> PortOutcome {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let event = port
                .next(Duration::from_millis(100))
                .expect("포트에서 꺼내기");
            if want(event) {
                return event;
            }
            assert!(Instant::now() < deadline, "10 초 안에 {what} 를 못 봤다");
        }
    }

    /// Drains the piled-up notifications until `Timeout` occurs.
    fn drain(port: &BirthPort) {
        for _ in 0..100 {
            if port
                .next(Duration::from_millis(300))
                .expect("포트에서 꺼내기")
                == PortOutcome::Timeout
            {
                return;
            }
        }
        panic!("알림이 그치지 않는다");
    }

    /// ⑥ Facts read through the pinned handle — parent = this test process that launched it · creation is later than this test process · executable ·
    /// command line (with the query right only). One pinned with the query right only cannot be ended.
    #[test]
    fn a_pinned_member_carries_its_facts_and_ends_only_with_the_kill_right() {
        let job = JobObjectHandle::new().expect("Job 생성");
        assert!(
            job.pin_member(std::process::id(), false)
                .expect("붙들기")
                .is_none(),
            "Job 밖의 이 시험 프로세스를 붙들었다"
        );
        assert!(
            job.pin_member(0, false).expect("붙들기").is_none(),
            "없는 번호를 붙들었다"
        );

        let mut x = spawn_gated_cmd("rem t40-facts", CREATE_NO_WINDOW.0);
        job.assign(x.id()).expect("Job 편입");

        let watcher = job
            .pin_member(x.id(), false)
            .expect("붙들기")
            .expect("우리 멤버");
        let facts = watcher.facts();
        assert_eq!(facts.ppid, std::process::id(), "부모: {facts:?}");
        let ProcessStart::Known(me) = process_start(std::process::id()) else {
            panic!("이 시험 프로세스의 생성 시각을 못 읽었다");
        };
        assert!(facts.create >= me, "생성이 부모보다 이르다: {facts:?}");
        assert!(
            facts.image.to_ascii_lowercase().ends_with("\\cmd.exe"),
            "실행 파일: {facts:?}"
        );
        assert!(
            facts.cmdline.contains("set /p _= & rem t40-facts"),
            "명령줄: {facts:?}"
        );
        assert!(!watcher.exited().expect("끝났나"));

        let refused = watcher.terminate_raw(EXIT_CODE);
        assert!(refused.is_err(), "끝내기 권한 없이 끝냈다");
        assert!(
            watcher.classify(refused).is_err(),
            "산 프로세스의 거절이 Gone 으로 읽혔다"
        );
        assert!(x.try_wait().expect("상태").is_none(), "조회 핸들이 죽였다");

        let killer = job
            .pin_member(x.id(), true)
            .expect("붙들기")
            .expect("우리 멤버");
        let raw = killer.terminate_raw(EXIT_CODE);
        assert_eq!(
            killer.classify(raw).expect("끝내기"),
            MemberOutcome::Terminated
        );
        assert!(
            watcher
                .wait_exit(Duration::from_secs(5))
                .expect("끝나기 대기"),
            "5 초 안에 안 끝났다"
        );
        assert!(watcher.exited().expect("끝났나"));
        let status = x.wait().expect("종료");
        assert_eq!(status.code(), Some(EXIT_CODE as i32));
    }

    /// ⑦-a If `start` fails the Job is not attached to the port — neither the re-announcement of the original member nor anything born in the Job afterwards
    /// arrives (had it been attached, both would already be piled up — ⑦-b).
    #[test]
    fn a_failed_start_leaves_the_job_unwatched() {
        let job = JobObjectHandle::new().expect("Job 생성");
        let mut x = spawn_gated_cmd("ping -n 30 127.0.0.1", CREATE_NO_WINDOW.0);
        job.assign(x.id()).expect("Job 편입");

        let mut held = None;
        let refused = job.watch_births(|port| {
            held = Some(port);
            Err(io::Error::other("기동 실패 흉내"))
        });
        assert_eq!(
            refused
                .expect_err("start 의 실패가 그대로 와야 한다")
                .to_string(),
            "기동 실패 흉내"
        );
        let port = held.expect("start 가 포트를 받았다");

        open_gate(&mut x);
        wait_until("Job 안의 ping", || {
            job.member_pids()
                .ok()?
                .into_iter()
                .find(|&pid| is_ping(pid))
        });
        let event = port.next(Duration::from_millis(500)).expect("꺼내기");
        assert_eq!(event, PortOutcome::Timeout, "붙지 않은 포트에 알림이 왔다");

        job.terminate(1).expect("Job 끝내기");
        let _ = x.wait();
    }

    /// ⑦-b An attached port — when attaching, the X already inside is announced again · the ping X launched is `Joined` · when that ping ends,
    /// `Other` · members born after detaching are not announced.
    #[test]
    fn a_watched_job_reports_births_until_unwatched() {
        let job = JobObjectHandle::new().expect("Job 생성");
        // There are two gates — the first ping on the first line, the second ping on the second line. The second gate is opened after detaching.
        let mut x = spawn_gated_cmd(
            "ping -n 30 127.0.0.1 & set /p _= & ping -n 30 127.0.0.1",
            CREATE_NO_WINDOW.0,
        );
        let root = x.id();
        job.assign(root).expect("Job 편입");
        let mut gate = x.stdin.take().expect("stdin 파이프");

        let mut held = None;
        job.watch_births(|port| {
            held = Some(port);
            Ok(())
        })
        .expect("붙이기");
        let port = held.expect("start 가 포트를 받았다");
        next_matching(&port, "X 의 되알림", |e| e == PortOutcome::Joined(root));

        gate.write_all(b"go\r\n").expect("첫 문");
        let PortOutcome::Joined(first) = next_matching(
            &port,
            "X 가 띄운 ping",
            |e| matches!(e, PortOutcome::Joined(pid) if is_ping(pid)),
        ) else {
            unreachable!("Joined 만 고른다")
        };
        // Hold it to the end — so the second ping cannot reuse this number.
        let ping = job
            .pin_member(first, true)
            .expect("붙들기")
            .expect("우리 멤버");
        assert_eq!(ping.facts().ppid, root, "ping 의 부모: {:?}", ping.facts());
        // Drain before ending — the `Other` below then comes from after the ending, and the only member left then is the waiting X, so there is no place for it to come from
        // other than the ping's ending.
        drain(&port);
        let raw = ping.terminate_raw(EXIT_CODE);
        assert_eq!(
            ping.classify(raw).expect("끝내기"),
            MemberOutcome::Terminated
        );
        next_matching(&port, "ping 의 끝남", |e| e == PortOutcome::Other);
        // Do not ask `exited()` right away — the ending notification may stand before the process object's signal (not measured).
        assert!(
            ping.wait_exit(Duration::from_secs(5)).expect("끝나기 대기"),
            "`Other` 가 왔는데 붙든 ping 이 끝나지 않았다"
        );

        job.unwatch_births().expect("떼기");
        drain(&port);

        gate.write_all(b"go\r\n").expect("둘째 문");
        let second = wait_until("뗀 뒤 태어난 ping", || {
            job.member_pids()
                .ok()?
                .into_iter()
                .find(|&pid| pid != first && is_ping(pid))
        });
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            let event = port.next(Duration::from_millis(100)).expect("꺼내기");
            assert!(
                !matches!(event, PortOutcome::Joined(_)),
                "뗀 뒤 태어난 {second} 이 알려졌다: {event:?}"
            );
        }

        job.terminate(1).expect("Job 끝내기");
        drop(gate);
        let _ = x.wait();
    }

    /// ① Even asking with a single first slot, growing and re-asking returns everything.
    #[test]
    fn a_one_slot_first_ask_still_lists_every_member() {
        let job = JobObjectHandle::new().expect("Job 생성");
        let mut root = spawn_gated_cmd("ping -n 30 127.0.0.1", CREATE_NO_WINDOW.0);
        job.assign(root.id()).expect("Job 편입");
        open_gate(&mut root);

        let mut full = wait_until("cmd 와 ping", || {
            let pids = job.member_pids().ok()?;
            pids.iter().any(|&pid| is_ping(pid)).then_some(pids)
        });
        let mut grown = job.member_pids_with_capacity(1).expect("늘려 되묻기");
        full.sort_unstable();
        grown.sort_unstable();
        assert!(
            grown.len() >= 2,
            "멤버가 둘 이상이어야 늘리기를 지난다: {grown:?}"
        );
        assert!(grown.contains(&root.id()), "뿌리가 빠졌다: {grown:?}");
        assert_eq!(grown, full);

        let _ = root.kill();
        let _ = root.wait();
    }

    /// ⑧ The three shapes of an MSYS command line — the hook-copy decision relies on them (TRD t40 §3-0): a script the shell exec'd has its own command line
    /// · its subshell (fork) has the same executable and command line as the parent · a `sleep` the script exec'd differs.
    /// The shell execs one command after passing the gate (`read _`) — handing it over without a fork like the hook's `-c "bash x.sh"`, so that
    /// everything is born inside the Job. Launched without a console so it does not drag in a console host (shell · script · subshell ·
    /// `sleep`: four).
    #[test]
    #[ignore = "Git Bash(C:\\Program Files\\Git\\usr\\bin\\bash.exe) 가 있어야 한다"]
    fn msys_births_carry_exec_fork_and_foreign_command_lines() {
        use std::os::windows::process::CommandExt;
        const BASH: &str = r"C:\Program Files\Git\usr\bin\bash.exe";
        assert!(
            std::path::Path::new(BASH).exists(),
            "{BASH} 가 없다 — PATH 의 bash 로 대신하지 않는다"
        );
        let script = std::env::temp_dir().join(format!("t40-facts-{}.sh", std::process::id()));
        // A non-interactive shell redirects a background command's stdin to /dev/null — the subshell lives to the end only if it receives the pipe as fd 3 and waits
        // on it.
        std::fs::write(
            &script,
            "exec 3<&0\n( read _ <&3 ) &\nexec /usr/bin/sleep 30\n",
        )
        .expect("스크립트 쓰기");
        let script_arg = script.to_string_lossy().replace('\\', "/");
        let script_name = script
            .file_name()
            .expect("파일 이름")
            .to_string_lossy()
            .into_owned();

        let job = JobObjectHandle::new().expect("Job 생성");
        // Call `/usr/bin` by absolute path — `usr\bin\bash.exe`, started without the Git launcher, has no `/usr/bin` in `PATH`
        // (measured).
        let mut shell = Command::new(BASH)
            .args(["-c", &format!("read _; exec /usr/bin/bash {script_arg}")])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(DETACHED_PROCESS.0)
            .spawn()
            .expect("bash 기동");
        let root = shell.id();
        job.assign(root).expect("Job 편입");
        // The subshell's `read _` waits on this pipe — hold it to the end.
        let mut gate = shell.stdin.take().expect("stdin 파이프");
        let shell_pin = job
            .pin_member(root, false)
            .expect("붙들기")
            .expect("우리 멤버");

        let mut held = None;
        job.watch_births(|port| {
            held = Some(port);
            Ok(())
        })
        .expect("붙이기");
        let port = held.expect("start 가 포트를 받았다");
        gate.write_all(b"go\n").expect("문");

        // Pin as soon as it is born — open the short-lived fork before it ends.
        let mut births: Vec<(u32, PinnedMember)> = Vec::new();
        let find = |births: &[(u32, PinnedMember)]| {
            let runs =
                |m: &PinnedMember, exe: &str| m.facts().image.to_ascii_lowercase().ends_with(exe);
            let (script_pid, script) = births
                .iter()
                .find(|(_, m)| m.facts().ppid == root && runs(m, "\\bash.exe"))?;
            let (_, fork) = births
                .iter()
                .find(|(_, m)| m.facts().ppid == *script_pid && runs(m, "\\bash.exe"))?;
            let (_, sleep) = births
                .iter()
                .find(|(_, m)| m.facts().ppid == *script_pid && runs(m, "\\sleep.exe"))?;
            Some((
                script.facts().clone(),
                fork.facts().clone(),
                sleep.facts().clone(),
            ))
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        let (script_facts, fork_facts, sleep_facts) = loop {
            if let Some(found) = find(&births) {
                break found;
            }
            assert!(
                Instant::now() < deadline,
                "10 초 안에 스크립트 · 서브셸 · sleep 을 다 못 봤다: {:#?}",
                births
                    .iter()
                    .map(|(pid, m)| (pid, m.facts()))
                    .collect::<Vec<_>>()
            );
            if let PortOutcome::Joined(pid) = port.next(Duration::from_millis(100)).expect("꺼내기")
            {
                if pid != root {
                    if let Some(member) = job.pin_member(pid, false).expect("붙들기") {
                        births.push((pid, member));
                    }
                }
            }
        };
        let shell_facts = shell_pin.facts().clone();

        assert!(
            shell_facts.cmdline.contains("exec /usr/bin/bash"),
            "껍데기: {shell_facts:?}"
        );
        assert!(
            script_facts.cmdline.contains(&script_name)
                && script_facts.cmdline != shell_facts.cmdline,
            "exec 한 스크립트가 제 명령줄을 갖지 않는다: {script_facts:?} / {shell_facts:?}"
        );
        assert_eq!(
            fork_facts.image, script_facts.image,
            "서브셸: {fork_facts:?}"
        );
        assert_eq!(
            fork_facts.cmdline, script_facts.cmdline,
            "서브셸이 스크립트의 명령줄 사본이 아니다"
        );
        assert!(
            sleep_facts.cmdline.contains("sleep") && sleep_facts.cmdline != script_facts.cmdline,
            "sleep 이 스크립트의 명령줄을 가졌다: {sleep_facts:?}"
        );

        // Detach before letting go of the port (the BirthPort rule).
        job.unwatch_births().expect("떼기");
        job.terminate(1).expect("Job 끝내기");
        drop(gate);
        let _ = shell.wait();
        let _ = std::fs::remove_file(&script);
    }
}
