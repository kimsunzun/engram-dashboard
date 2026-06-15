//! Windows Job Object 래퍼.
//!
//! 자식 PTY 프로세스를 Job에 묶어, 우리가 명시적으로 죽이거나(TerminateJobObject)
//! Tauri 프로세스가 크래시될 때(KILL_ON_JOB_CLOSE) 손자 프로세스까지 함께 정리한다.
//!
//! 호출 순서/플래그는 Phase 0 spike(examples/spike.rs)에서 Windows 실측 검증한 것과 동일하다.
//! 이 파일은 platform 전용이라 windows crate import는 허용되지만, tauri import는 0개여야 한다.

use std::io;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};

/// Windows Job Object 핸들 래퍼. Drop 시 CloseHandle 한다.
pub struct JobObjectHandle {
    handle: HANDLE,
}

// SAFETY: HANDLE은 raw 포인터 wrapper라 자동으로 Send/Sync가 아니다.
// Job 핸들은 우리가 생성/종료/CloseHandle 까지 소유권을 단독으로 관리하며,
// 동시 변이가 없으므로(생성 후 read-only로 assign/terminate 호출) 스레드 간 이동/공유를 허용한다.
unsafe impl Send for JobObjectHandle {}
unsafe impl Sync for JobObjectHandle {}

impl JobObjectHandle {
    /// Job 생성 + KILL_ON_JOB_CLOSE 설정.
    pub fn new() -> io::Result<Self> {
        // SAFETY: CreateJobObjectW — 익명(이름 없는) Job 생성. 인자 모두 None이라
        // 보안 속성/이름 없이 새 Job 핸들을 만든다. 실패 시 Err 반환.
        let handle = unsafe { CreateJobObjectW(None, None) }.map_err(win_err)?;

        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        // Job 핸들이 닫히면(우리가 명시적으로 죽이지 않아도, 예: Tauri 프로세스 크래시)
        // Job 내 모든 프로세스를 강제 종료하도록 플래그 설정 → 손자 프로세스 누수 방지.
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
            // 설정 실패 시 누수 방지를 위해 핸들을 닫고 에러 반환.
            // SAFETY: 방금 생성한 유효한 핸들을 한 번만 닫는다.
            unsafe {
                let _ = CloseHandle(handle);
            }
            return Err(win_err(e));
        }

        Ok(Self { handle })
    }

    /// child process를 이 Job에 편입. process id로 OpenProcess → AssignProcessToJobObject.
    pub fn assign(&self, process_id: u32) -> io::Result<()> {
        // SAFETY: OpenProcess — Job 편입에 필요한 최소 권한(SET_QUOTA|TERMINATE)으로
        // 대상 pid의 프로세스 핸들을 연다. 핸들 상속(false). 실패 시 Err.
        let process =
            unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, false, process_id) }
                .map_err(win_err)?;

        // SAFETY: AssignProcessToJobObject — 유효한 Job 핸들과 방금 연 프로세스 핸들.
        let result = unsafe { AssignProcessToJobObject(self.handle, process) };

        // 프로세스 핸들은 assign 후 더 이상 필요 없다(Job은 별도). 즉시 닫는다.
        // SAFETY: OpenProcess가 반환한 유효한 핸들을 한 번만 닫는다.
        unsafe {
            let _ = CloseHandle(process);
        }

        result.map_err(win_err)
    }

    /// Job 내 전 프로세스 강제 종료.
    pub fn terminate(&self, exit_code: u32) -> io::Result<()> {
        // SAFETY: TerminateJobObject — 유효한 Job 핸들. Job에 편입된 모든 프로세스를
        // 지정 exit code로 강제 종료한다.
        unsafe { TerminateJobObject(self.handle, exit_code) }.map_err(win_err)
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

/// windows::core::Error → std::io::Error 변환.
/// HRESULT를 from_raw_os_error에 넘기면 0x8007xxxx로 래핑돼 메시지가 깨지므로
/// io::Error::other로 원본 에러(메시지 포함)를 보존한다.
fn win_err(e: windows::core::Error) -> io::Error {
    io::Error::other(e)
}
