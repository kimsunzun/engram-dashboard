//! Windows Job Object 래퍼 + 멈춘 채 띄운 프로세스를 깨우는 도구([`resume_suspended_process`]).
//!
//! 자식 프로세스를 Job에 묶어, 우리가 명시적으로 죽이거나(TerminateJobObject)
//! 호스트 프로세스가 크래시될 때(KILL_ON_JOB_CLOSE) 손자 프로세스까지 함께 정리한다.
//!
//! ★Job 은 그 안에 든 **뒤에** 만들어진 프로세스에만 물려진다★ — 이미 뜬 프로세스를 [`JobObjectHandle::assign`]
//!   으로 넣으면, 넣기 전에 그것이 띄운 자식은 Job 밖에 남아 트리 kill 을 빠져나간다. 사용량 조회(`usage::process`)
//!   는 멈춘 채(`CREATE_SUSPENDED`) 띄워 넣은 뒤 깨워 그 틈을 닫았다. ★에이전트 통로(`transport::pty`·
//!   `transport::stdio`·codex 통로)는 띄운 뒤에 넣으므로 그 틈을 그대로 안고 있다★.
//!
//! 호출 순서/플래그는 Phase 0 spike(examples/spike.rs)에서 Windows 실측 검증한 것과 동일하다.
//! 이 파일은 platform 전용이라 windows crate import는 허용되지만, tauri import는 0개여야 한다.

use std::io;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicAccountingInformation,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
    TerminateJobObject, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows::Win32::System::Threading::{
    GetProcessIdOfThread, OpenProcess, OpenThread, ResumeThread, PROCESS_SET_QUOTA,
    PROCESS_TERMINATE, THREAD_QUERY_LIMITED_INFORMATION, THREAD_SUSPEND_RESUME,
};

pub struct JobObjectHandle {
    handle: HANDLE,
}

// SAFETY: HANDLE은 raw 포인터 wrapper라 자동으로 Send/Sync가 아니다.
// Job 핸들은 우리가 생성/종료/CloseHandle 까지 소유권을 단독으로 관리하며,
// 동시 변이가 없으므로(생성 후 read-only로 assign/terminate 호출) 스레드 간 이동/공유를 허용한다.
unsafe impl Send for JobObjectHandle {}
unsafe impl Sync for JobObjectHandle {}

impl JobObjectHandle {
    pub fn new() -> io::Result<Self> {
        // SAFETY: CreateJobObjectW — 인자 모두 None(보안 속성·이름 없는 익명 Job).
        let handle = unsafe { CreateJobObjectW(None, None) }.map_err(win_err)?;

        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

        // SAFETY: SetInformationJobObject — 위에서 만든 유효한 Job 핸들에
        // 스택에 있는 info 구조체 포인터와 정확한 크기를 넘긴다. 클래스와 구조체 타입이 일치(Extended).
        let result = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                core::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if let Err(e) = result {
            // SAFETY: 방금 생성한 유효한 핸들을 한 번만 닫는다.
            unsafe {
                let _ = CloseHandle(handle);
            }
            return Err(win_err(e));
        }

        Ok(Self { handle })
    }

    /// ★띄운 뒤에 넣으면 넣기 전에 그것이 띄운 자식은 Job 밖에 남는다★(에이전트 통로 셋의 틈) — 닫는 선례 =
    /// 멈춘 채 띄워 넣은 뒤 [`resume_suspended_process`] 로 깨우는 `usage::process`.
    pub fn assign(&self, process_id: u32) -> io::Result<()> {
        // SAFETY: OpenProcess — AssignProcessToJobObject 가 요구하는 최소 권한
        // (SET_QUOTA|TERMINATE)만 연다.
        let process =
            unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, false, process_id) }
                .map_err(win_err)?;

        // SAFETY: AssignProcessToJobObject — 유효한 Job 핸들과 방금 연 프로세스 핸들.
        let result = unsafe { AssignProcessToJobObject(self.handle, process) };

        // Job 편입은 프로세스 핸들과 무관하게 유지되므로 여기서 닫아도 안전하다.
        // SAFETY: OpenProcess가 반환한 유효한 핸들을 한 번만 닫는다.
        unsafe {
            let _ = CloseHandle(process);
        }

        result.map_err(win_err)
    }

    pub fn terminate(&self, exit_code: u32) -> io::Result<()> {
        // SAFETY: TerminateJobObject — 유효한 Job 핸들. Job에 편입된 모든 프로세스를
        // 지정 exit code로 강제 종료한다.
        unsafe { TerminateJobObject(self.handle, exit_code) }.map_err(win_err)
    }

    /// Job 안에서 아직 끝나지 않은 프로세스 수. `terminate` 는 종료를 시작만 하고 돌아오므로, 「트리가 다
    /// 끝났다」를 기다리는 쪽이 이것으로 확인한다.
    pub fn active_processes(&self) -> io::Result<u32> {
        let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        // SAFETY: QueryInformationJobObject — 유효한 Job 핸들에 클래스(BasicAccounting)와 맞는 스택
        // 구조체 포인터와 정확한 크기를 넘긴다. 반환 길이는 받지 않는다(None).
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
}

impl Drop for JobObjectHandle {
    fn drop(&mut self) {
        // 핸들이 닫히면 KILL_ON_JOB_CLOSE 덕에 잔여 프로세스도 OS가 정리한다.
        // SAFETY: new()에서 생성한 유효한 Job 핸들을 Drop 시 한 번만 닫는다.
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

/// `CREATE_SUSPENDED` 로 띄운 프로세스 `process_id` 의 멈춘 스레드를 깨운다(막 띄운 프로세스는 첫 스레드 하나다).
///
/// std 의 `Child` 는 첫 스레드 핸들을 내주지 않는다(기동 직후 닫는다) — 그래서 스레드 스냅숏에서 그 프로세스의
/// 스레드를 찾는다. 스냅숏 뒤에 사라진 스레드와 그 번호를 이어받은 남의 스레드는 건너뛴다(깨우지 않는다).
///
/// ★멈춰 있던 스레드를 하나도 못 깨웠으면 오류다★ — 멈추지 않은 프로세스에 부르면 실패한다. 그래서 띄우는 쪽이
///   멈춘 채 띄우기를 잃으면(Job 에 넣기 전에 자식이 돌기 시작한다) 기동이 조용히 틈을 되살리지 않고 실패한다.
///   그 프로세스 자신의 스레드를 못 깨우면(`ResumeThread` 실패) 그 오류다.
pub fn resume_suspended_process(process_id: u32) -> io::Result<()> {
    // SAFETY: 스레드 스냅숏은 시스템 전체라 프로세스 인자는 무시된다(0).
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }.map_err(win_err)?;
    let mut entry = THREADENTRY32 {
        dwSize: core::mem::size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };
    let mut resumed = 0usize;
    let mut seen = 0usize;
    let mut failure = None;
    // SAFETY: 유효한 스냅숏 핸들 + dwSize 를 채운 스택 entry.
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
        // SAFETY: 같은 유효 스냅숏 핸들 + entry.
        more = unsafe { Thread32Next(snapshot, &mut entry) }.is_ok();
    }
    // SAFETY: 스냅숏 핸들을 한 번만 닫는다.
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
    /// 멈춰 있던 스레드의 멈춤 횟수를 하나 내렸다.
    WasSuspended,
    /// 이미 돌던 스레드다 — `ResumeThread` 는 아무것도 바꾸지 않았다.
    NotSuspended,
    /// 스냅숏 뒤에 사라졌거나 그 번호가 남의 프로세스로 넘어갔다 — 건드리지 않았다.
    NotOurs,
}

fn resume_own_thread(thread_id: u32, process_id: u32) -> io::Result<Resumed> {
    // SAFETY: OpenThread — ResumeThread·GetProcessIdOfThread 가 요구하는 권한만 연다.
    let opened = unsafe {
        OpenThread(
            THREAD_SUSPEND_RESUME | THREAD_QUERY_LIMITED_INFORMATION,
            false,
            thread_id,
        )
    };
    // 열리지 않는 스레드 = 스냅숏 뒤에 끝났다(그 밖의 사유도 여기 든다 — 깨운 것이 없으면 부르는 쪽이 실패한다).
    let Ok(thread) = opened else {
        return Ok(Resumed::NotOurs);
    };
    // SAFETY: 방금 연 유효한 스레드 핸들. 실패하면 0 이다 — 0 은 System Idle 프로세스의 번호라 우리가 띄운
    // 자식의 번호일 수 없고, 그래서 아래 비교에서 「남의 것」으로 갈린다.
    let owner = unsafe { GetProcessIdOfThread(thread) };
    let result = if owner != process_id {
        Ok(Resumed::NotOurs)
    } else {
        // SAFETY: 같은 유효 핸들. 돌려주는 값 = 부르기 전의 멈춤 횟수, 실패 = `u32::MAX`(문서화된 `(DWORD)-1`).
        match unsafe { ResumeThread(thread) } {
            // 마지막 오류는 CloseHandle 이 덮어쓰기 전에 읽는다.
            u32::MAX => Err(io::Error::last_os_error()),
            0 => Ok(Resumed::NotSuspended),
            _ => Ok(Resumed::WasSuspended),
        }
    };
    // SAFETY: OpenThread 가 돌려준 유효 핸들을 한 번만 닫는다.
    unsafe {
        let _ = CloseHandle(thread);
    }
    result
}

/// HRESULT를 from_raw_os_error에 넘기면 0x8007xxxx로 래핑돼 메시지가 깨지므로
/// io::Error::other로 원본 에러(메시지 포함)를 보존한다.
fn win_err(e: windows::core::Error) -> io::Error {
    io::Error::other(e)
}
