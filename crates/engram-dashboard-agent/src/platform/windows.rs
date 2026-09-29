//! Windows Job Object 래퍼.
//!
//! 통로가 띄운 자식 프로세스를 Job에 묶어, 우리가 명시적으로 죽이거나(TerminateJobObject)
//! 호스트 프로세스가 크래시될 때(KILL_ON_JOB_CLOSE) 손자 프로세스까지 함께 정리한다.
//! 멤버 하나씩 다루는 조각(명단 · 검증 · 끝내기)도 여기다 — 무엇을 끝낼지는 모르고, 고르는 쪽이 준 신원만
//! 확인한다(ADR-0257).
//!
//! 호출 순서/플래그는 Phase 0 spike(examples/spike.rs)에서 Windows 실측 검증한 것과 동일하다.
//! 이 파일은 platform 전용이라 windows crate import는 허용되지만, tauri import는 0개여야 한다.
//! ★`windows` crate 와 `io` 말고는 쓰지 않는다★ — 결과 타입도 여기 두고 중립 손잡이(`process_group`)가
//! 감싼다. 그래야 이 파일을 별도 플랫폼 모듈로 통째 옮길 수 있다(사용자 결정 2026-09-29).

use std::io;

use windows::core::HRESULT;
use windows::Win32::Foundation::{
    CloseHandle, BOOL, ERROR_INVALID_PARAMETER, ERROR_MORE_DATA, FILETIME, HANDLE, WAIT_FAILED,
    WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob, JobObjectBasicProcessIdList,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
    TerminateJobObject, JOBOBJECT_BASIC_PROCESS_ID_LIST, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, TerminateProcess, WaitForSingleObject,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
};

/// 끊기 뒤 잔여물 정리가 끝낸 프로세스의 종료 코드 — 사후 조사에서 종료 코드만으로 그 정리가 끝낸 것을 알아본다.
/// `shutdown()` 의 Job 통째 끝내기(1)와 겹치지 않게 골랐다.
// ADR-0257
pub(crate) const LEFTOVER_EXIT_CODE: u32 = 0x7440;

/// 운영에서 명단을 처음 물을 때의 칸 수. 모자라면 늘린다.
const MEMBER_LIST_INITIAL: usize = 64;
/// 명단 되묻기 상한 — 묻는 사이 멤버가 계속 늘어 완전한 명단을 못 받으면 여기서 멈춘다(닫힌 실패).
const MEMBER_LIST_ATTEMPTS: usize = 4;

/// [`JobObjectHandle::verify_member`] 의 답. `Ready` 가 아니면 끝내지 않는다.
///
/// - `Gone` = 그 번호의 프로세스가 없거나 이미 끝났다.
/// - `NotOurs` = 산 프로세스지만 이 Job 밖이거나 시작시각이 달라 그 신원이 아니다.
// ADR-0257
#[derive(Debug)]
pub(crate) enum MemberCheck {
    Ready(VerifiedMember),
    Gone,
    NotOurs,
}

/// [`VerifiedMember::settle`] 의 답. `Gone` = 검증 뒤 끝내기 전에 스스로 끝났다.
// ADR-0257
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MemberOutcome {
    Terminated,
    Gone,
}

/// 검증을 마친 프로세스 핸들 — 쥐고 있는 동안 그 PID 는 재사용되지 않는다(열린 핸들이 프로세스 객체를
/// 붙든다). drop = CloseHandle.
// ADR-0257
#[derive(Debug)]
pub(crate) struct VerifiedMember {
    handle: HANDLE,
}

// SAFETY: 프로세스 핸들은 어느 스레드에서 써도 된다. 이 값이 단독 소유하고 닫기는 drop 한 번뿐이다.
unsafe impl Send for VerifiedMember {}

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

    /// 지금 이 Job 에 든 프로세스 PID 전부(중첩 Job 의 멤버 포함 · 순서는 뜻이 없다).
    ///
    /// ★완전할 때만 `Ok`★ — 되묻기 상한 안에 명단 수가 Job 의 멤버 수와 같아지지 않으면 `Err` 다. 빠진 멤버는
    /// 호출자에게 「우리 것이 아니다」로 읽힌다.
    // ADR-0257
    pub(crate) fn member_pids(&self) -> io::Result<Vec<u32>> {
        self.member_pids_with_capacity(MEMBER_LIST_INITIAL)
    }

    /// [`Self::member_pids`] 와 같은 규칙 · 첫 칸 수만 주입한다 — 시험이 늘리기 · 되묻기 경로를 적은 프로세스로 잰다.
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
            // usize 칸으로 잡아 구조체 정렬을 맞춘다 — 머리 두 u32 뒤에 ULONG_PTR 항목이 온다.
            let mut buf = vec![0usize; HEAD_WORDS + capacity];
            let bytes = u32::try_from(core::mem::size_of_val(buf.as_slice())).map_err(|_| {
                io::Error::other(format!("Job 멤버 명단 버퍼가 너무 크다({capacity} 칸)"))
            })?;
            // SAFETY: 유효한 Job 핸들 + 쓰기 가능한 `bytes` 바이트 버퍼(usize 정렬 · 머리 + 항목 1 칸 이상).
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
            // SAFETY: buf 는 구조체 하나 크기 이상이고 usize 정렬이다. 필드를 참조 없이 값으로 읽는다.
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
            // 머리를 못 채운 답(멤버 수 0)이어도 칸은 늘어야 되묻기가 뜻이 있다.
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

    /// 그 신원이 아직 **이 Job 의 산 멤버**인지 같은 핸들로 본다. 끝내지 않는다.
    ///
    /// `Err` = 여는 실패(부재 말고) · Job 소속 · 시작시각 · 대기 조회 자체의 실패 — 무엇이든 끝내기로 가지 않는다.
    // ADR-0257
    pub(crate) fn verify_member(&self, pid: u32, expected_start: u64) -> io::Result<MemberCheck> {
        // SAFETY: 끝내기 · 시각 조회 · 대기에 드는 권한만 연다.
        let handle = match unsafe {
            OpenProcess(
                PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                false,
                pid,
            )
        } {
            Ok(h) => h,
            Err(e) if e.code() == HRESULT::from_win32(ERROR_INVALID_PARAMETER.0) => {
                return Ok(MemberCheck::Gone)
            }
            Err(e) => return Err(win_err(e)),
        };
        let member = VerifiedMember { handle };

        let mut in_job = BOOL::default();
        // SAFETY: 두 유효 핸들 + 스택 출력. ★Job 인자는 늘 우리 핸들이다★ — NULL 이면 「아무 Job 에나
        //   들었나」로 바뀌어 남의 Job 멤버를 우리 것으로 본다.
        unsafe { IsProcessInJob(member.handle, self.handle, &mut in_job) }.map_err(win_err)?;
        if !in_job.as_bool() {
            return Ok(MemberCheck::NotOurs);
        }

        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        // SAFETY: 유효 핸들 + 스택의 4개 FILETIME 출력 포인터.
        unsafe {
            GetProcessTimes(
                member.handle,
                &mut creation,
                &mut exit,
                &mut kernel,
                &mut user,
            )
        }
        .map_err(win_err)?;
        let start = ((creation.dwHighDateTime as u64) << 32) | (creation.dwLowDateTime as u64);
        if start != expected_start {
            return Ok(MemberCheck::NotOurs);
        }

        // 생존을 종료 코드(STILL_ACTIVE = 259)로 재지 않는다 — 259 로 끝난 프로세스가 산 것으로 보인다.
        // SAFETY: 유효 핸들 · 기다리지 않는 조회.
        match unsafe { WaitForSingleObject(member.handle, 0) } {
            WAIT_OBJECT_0 => Ok(MemberCheck::Gone),
            WAIT_TIMEOUT => Ok(MemberCheck::Ready(member)),
            WAIT_FAILED => Err(io::Error::last_os_error()),
            other => Err(io::Error::other(format!(
                "프로세스 대기가 뜻밖의 값 {:#x} 를 돌려줬다",
                other.0
            ))),
        }
    }
}

impl VerifiedMember {
    /// 검증한 그 핸들로 `TerminateProcess` **호출 하나만** 한다. 비동기 — 대상이 끝나기를 기다리지 않는다.
    /// 날것 결과만 돌려주며 가르기와 닫기는 [`Self::settle`] 이 한다(자물쇠 안에서 부를 수 있게 좁혔다).
    // ADR-0257
    pub(crate) fn terminate_raw(&self, exit_code: u32) -> io::Result<()> {
        // ★실패도 힙을 쓰지 않는 오류로 만든다★ — 자물쇠 안에서 불리므로 `win_err`(`io::Error::other` = 상자 할당)를
        //   쓰지 않는다. windows 0.58 의 `ok()` 는 `GetLastError` 를 읽기만 해 마지막 오류가 그대로 남아 있다.
        // SAFETY: 검증 때 PROCESS_TERMINATE 로 연 유효 핸들.
        unsafe { TerminateProcess(self.handle, exit_code) }.map_err(|_| io::Error::last_os_error())
    }

    /// [`Self::terminate_raw`] 의 날것 결과를 가르고 핸들을 닫는다. 실패여도 그 사이 스스로 끝났으면 `Gone` 이다.
    // ADR-0257
    pub(crate) fn settle(self, raw: io::Result<()>) -> io::Result<MemberOutcome> {
        match raw {
            Ok(()) => Ok(MemberOutcome::Terminated),
            // SAFETY: 유효 핸들 · 기다리지 않는 조회.
            Err(_) if unsafe { WaitForSingleObject(self.handle, 0) } == WAIT_OBJECT_0 => {
                Ok(MemberOutcome::Gone)
            }
            Err(e) => Err(e),
        }
    }
}

impl Drop for VerifiedMember {
    fn drop(&mut self) {
        // SAFETY: verify_member 가 연 유효 핸들을 한 번만 닫는다.
        unsafe {
            let _ = CloseHandle(self.handle);
        }
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

/// HRESULT를 from_raw_os_error에 넘기면 0x8007xxxx로 래핑돼 메시지가 깨지므로
/// io::Error::other로 원본 에러(메시지 포함)를 보존한다.
fn win_err(e: windows::core::Error) -> io::Error {
    io::Error::other(e)
}
