//! Windows Job Object 래퍼.
//!
//! 통로가 띄운 자식 프로세스를 Job에 묶어, 우리가 명시적으로 죽이거나(TerminateJobObject)
//! 호스트 프로세스가 크래시될 때(KILL_ON_JOB_CLOSE) 손자 프로세스까지 함께 정리한다.
//! 멤버 하나씩 다루는 조각(명단 · 붙들기와 사실 · 끝내기)과 Job 가입 알림 포트도 여기다 — 무엇을 끝낼지는
//! 모르고, 고르는 쪽이 준 번호가 이 Job 의 멤버인지만 확인해 붙든다(ADR-0257).
//!
//! 호출 순서/플래그는 Phase 0 spike(examples/spike.rs)에서 Windows 실측 검증한 것과 동일하다.
//! 이 파일은 platform 전용이라 windows crate import는 허용되지만, tauri import는 0개여야 한다.
//! ★`windows` crate 와 표준 라이브러리 말고는 쓰지 않는다★ — 결과 · 사실 · 알림 타입도 여기 두고 중립
//! 손잡이(`process_group`)가 제 타입으로 옮겨 감싼다. 그래야 이 파일을 별도 플랫폼 모듈로 통째 옮길 수 있다(사용자
//! 결정 2026-09-29). 시험만 바닥 crate(`engram_dashboard_base`)의 시작 시각 조회를 빌린다.

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
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob,
    JobObjectAssociateCompletionPortInformation, JobObjectBasicProcessIdList,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
    TerminateJobObject, JOBOBJECT_ASSOCIATE_COMPLETION_PORT, JOBOBJECT_BASIC_PROCESS_ID_LIST,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, TerminateProcess,
    WaitForSingleObject, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA,
    PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
};
use windows::Win32::System::IO::{CreateIoCompletionPort, GetQueuedCompletionStatus, OVERLAPPED};

/// 끊기 뒤 잔여물 정리가 끝낸 프로세스의 종료 코드 — 사후 조사에서 종료 코드만으로 그 정리가 끝낸 것을 알아본다.
/// `shutdown()` 의 Job 통째 끝내기(1)와 겹치지 않게 골랐다.
// ADR-0257
pub(crate) const LEFTOVER_EXIT_CODE: u32 = 0x7440;

/// 운영에서 명단을 처음 물을 때의 칸 수. 모자라면 늘린다.
const MEMBER_LIST_INITIAL: usize = 64;
/// 명단 되묻기 상한 — 묻는 사이 멤버가 계속 늘어 완전한 명단을 못 받으면 여기서 멈춘다(닫힌 실패).
const MEMBER_LIST_ATTEMPTS: usize = 4;

/// 완료 포트 패킷의 바이트 수 칸에 실려 오는 Job 알림 번호 — 멤버가 새로 들었다. 이 상수 하나 때문에
/// `Win32_System_SystemServices` 를 켜지 않으려 여기 둔다.
const JOB_OBJECT_MSG_NEW_PROCESS: u32 = 6;
/// Job 을 포트에 붙일 때 주는 키 — 이 키로 온 패킷만 Job 알림으로 읽는다.
const BIRTH_PORT_KEY: usize = 1;
/// 실행 파일 경로 버퍼(UTF-16 칸) — Win32 경로의 최대 길이.
const IMAGE_PATH_CHARS: usize = 32_768;
/// 명령줄 버퍼 — 머리(`UNICODE_STRING`) 뒤에 가장 긴 명령줄(32 767 자)이 들어가는 크기. 모자라면 명령줄은 빈 칸이다.
const COMMAND_LINE_BYTES: usize = core::mem::size_of::<UNICODE_STRING>() + 65_536;

/// `NtQueryInformationProcess(ProcessBasicInformation)` 의 결과 배치 — `Win32_System_Kernel` 을 켜지 않으려
/// 여기 둔다. 포인터 칸은 같은 크기 · 정렬의 `usize` 로 받는다.
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

/// [`JobObjectHandle::pin_member`] 가 붙든 핸들로 한 번 읽은 사실. 칸의 뜻 · 못 읽은 칸의 값은 중립 손잡이의 사실
/// 타입과 같다 — 이 파일이 중립 손잡이를 부르지 않으려 제 모양을 따로 둔다(모듈 헤더).
#[derive(Debug, Clone, Default)]
pub(crate) struct MemberFacts {
    pub ppid: u32,
    pub create: u64,
    pub image: String,
    pub cmdline: String,
}

/// [`BirthPort::next`] 의 답 — 뜻은 중립 손잡이의 알림 타입과 같다(따로 두는 이유도 [`MemberFacts`] 와 같다).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PortOutcome {
    Joined(u32),
    Other,
    Timeout,
}

/// 우리 Job 멤버로 확인하고 붙든 프로세스 — 쥐는 동안 그 PID 는 재사용되지 않는다(열린 핸들이 프로세스 객체를
/// 붙든다). 끝난 뒤에도 쥘 수 있다. drop = CloseHandle.
#[derive(Debug)]
pub(crate) struct PinnedMember {
    handle: HANDLE,
    facts: MemberFacts,
}

// SAFETY: 프로세스 핸들은 어느 스레드에서 써도 된다. 메서드는 핸들로 OS 조회 · 끝내기만 하고, 닫기는 이 값이
//   단독 소유한 drop 한 번뿐이다.
unsafe impl Send for PinnedMember {}
unsafe impl Sync for PinnedMember {}

/// Job 가입 알림 포트 — [`JobObjectHandle::watch_births`] 가 만든다. 여러 스레드가 `Arc` 로 나눠 쥐고, 마지막
/// drop 이 포트 핸들을 닫는다.
///
/// 알림은 통지일 뿐 배달이 보장되지 않는다(MS Learn) — 빠진 가입은 여기서 알 수 없다.
///
/// ★Job 이 붙은 채 마지막 `Arc` 를 놓지 않는다 — 먼저 떼고([`JobObjectHandle::unwatch_births`]) 놓는다★: 붙어 있는
/// 동안 Job 이 포트를 제 커널 참조로 쥐어 핸들이 닫혀도 포트가 남고, 아무도 안 읽는 포트에 알림이 계속 쌓인다.
/// 그 커널 참조는 MS Learn 에 적혀 있지 않고 여기서 실측하지 않았다 — 틀려도 먼저 떼는 쪽은 잃는 것이 없다.
#[derive(Debug)]
pub(crate) struct BirthPort {
    handle: HANDLE,
}

// SAFETY: 완료 포트 핸들은 여러 스레드가 동시에 꺼내도 되는 커널 객체다. 닫기는 마지막 `Arc` 의 drop 한 번뿐이다.
unsafe impl Send for BirthPort {}
unsafe impl Sync for BirthPort {}

/// [`PinnedMember::classify`] 의 답. `Gone` = 붙든 뒤 끝내기 전에 스스로 끝났다.
// ADR-0257
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MemberOutcome {
    Terminated,
    Gone,
}

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

    /// 그 번호의 프로세스를 열어 **이 Job 의 멤버일 때만** 붙들고 사실을 한 번 읽는다. 살았는지는 보지 않는다
    /// ([`PinnedMember::exited`]).
    ///
    /// `kill` = 끝내기 권한도 연다 — `false` 로 붙든 것은 [`PinnedMember::terminate_raw`] 가 늘 실패한다.
    /// `Ok(None)` = 그 번호의 프로세스가 없다 · 이 Job 밖이다. `Err` = 여는 실패(부재 말고 — 권한 거부 등) ·
    /// 소속 조회 실패.
    pub(crate) fn pin_member(&self, pid: u32, kill: bool) -> io::Result<Option<PinnedMember>> {
        let mut access = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE;
        if kill {
            access |= PROCESS_TERMINATE;
        }
        // SAFETY: 조회 · 대기(· 끝내기) 권한만 연다.
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
        // SAFETY: 두 유효 핸들 + 스택 출력. ★Job 인자는 늘 우리 핸들이다★ — NULL 이면 「아무 Job 에나
        //   들었나」로 바뀌어 남의 Job 멤버를 우리 것으로 본다.
        unsafe { IsProcessInJob(member.handle, self.handle, &mut in_job) }.map_err(win_err)?;
        if !in_job.as_bool() {
            return Ok(None);
        }
        member.facts = read_facts(member.handle);
        Ok(Some(member))
    }

    /// 가입 알림 포트를 만들어 `start` 에 넘기고, **`start` 가 `Ok` 일 때만** 이 Job 을 그 포트에 붙인다 — 알림이
    /// 쌓일 수 있게 되는 때에는 꺼낼 쪽이 이미 포트를 쥐고 있다. `start` 의 `Err` 는 그대로 돌려준다(붙지 않았다).
    ///
    /// 붙일 때 이미 든 멤버도 `Joined` 로 다시 알려진다 · 붙이는 동안 상태가 바뀐 멤버의 알림은 빠질 수 있다
    /// (MS Learn). ★한 Job 에 한 번만 부른다★ — 두 번째 부름은 여기서 막지 않는다(부르는 쪽 몫).
    pub(crate) fn watch_births(
        &self,
        start: impl FnOnce(Arc<BirthPort>) -> io::Result<()>,
    ) -> io::Result<()> {
        // SAFETY: 파일 없이 새 포트만 만든다(INVALID_HANDLE_VALUE · 기존 포트 없음 · 동시 실행 1).
        let handle =
            unsafe { CreateIoCompletionPort(INVALID_HANDLE_VALUE, HANDLE::default(), 0, 1) }
                .map_err(win_err)?;
        let port = Arc::new(BirthPort { handle });
        start(Arc::clone(&port))?;
        let attached = self.associate_port(port.handle);
        // 붙이기가 끝난 뒤에 놓는다 — `start` 쪽이 먼저 놓아 핸들이 닫히면 그 값이 재사용돼 남의 핸들을 붙일 수 있다.
        drop(port);
        attached
    }

    /// 이 Job 의 포트 연결을 뗀다(포트 자리에 NULL — MS Learn 이 적은 떼기). 뗀 뒤 태어난 멤버는 알려지지 않는다.
    pub(crate) fn unwatch_births(&self) -> io::Result<()> {
        self.associate_port(HANDLE::default())
    }

    fn associate_port(&self, port: HANDLE) -> io::Result<()> {
        let info = JOBOBJECT_ASSOCIATE_COMPLETION_PORT {
            CompletionKey: core::ptr::without_provenance_mut(BIRTH_PORT_KEY),
            CompletionPort: port,
        };
        // SAFETY: 유효한 Job 핸들 + 스택 구조체 포인터와 정확한 크기. 클래스와 구조체 타입이 일치한다.
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
    /// 지금 끝났나 — 기다리지 않는다. `Err` = 대기 조회 자체의 실패.
    pub(crate) fn exited(&self) -> io::Result<bool> {
        signaled_within(self.handle, 0)
    }

    /// `limit` 까지 끝나기를 기다린다 — 끝났으면 `true`. `Err` = 대기 조회 자체의 실패.
    pub(crate) fn wait_exit(&self, limit: Duration) -> io::Result<bool> {
        signaled_within(self.handle, finite_millis(limit))
    }

    pub(crate) fn facts(&self) -> &MemberFacts {
        &self.facts
    }

    /// 붙든 그 핸들로 [`LEFTOVER_EXIT_CODE`] 를 주는 `TerminateProcess` **호출 하나만** 한다. 비동기 — 대상이
    /// 끝나기를 기다리지 않는다. 날것 결과만 돌려주며 가르기는 [`Self::classify`] 가 한다(자물쇠 안에서 부를 수
    /// 있게 좁혔다). `kill = false` 로 붙든 것은 늘 `Err` 다.
    pub(crate) fn terminate_raw(&self) -> io::Result<()> {
        terminate_handle(self.handle, LEFTOVER_EXIT_CODE)
    }

    /// [`Self::terminate_raw`] 의 날것 결과를 가른다. 실패여도 그 사이 스스로 끝났으면 `Gone` 이다. 핸들은 닫지
    /// 않는다(drop 이 닫는다).
    pub(crate) fn classify(&self, raw: io::Result<()>) -> io::Result<MemberOutcome> {
        classify_terminate(self.handle, raw)
    }
}

impl BirthPort {
    /// 알림 하나를 `wait` 까지 기다려 꺼낸다. `Err` = 꺼내기 자체의 실패(포트가 닫힘 등).
    pub(crate) fn next(&self, wait: Duration) -> io::Result<PortOutcome> {
        let mut message = 0u32;
        let mut key = 0usize;
        let mut overlapped: *mut OVERLAPPED = core::ptr::null_mut();
        // SAFETY: 유효 포트 핸들 + 스택 출력 셋 · 유한 대기.
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
                // Job 알림은 OVERLAPPED 칸에 포인터가 아니라 PID 를 싣는다 — 역참조하지 않는다.
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
        // SAFETY: pin_member 가 연 유효 핸들을 한 번만 닫는다.
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

impl Drop for BirthPort {
    fn drop(&mut self) {
        // SAFETY: watch_births 가 만든 유효 포트 핸들을 마지막 `Arc` 가 한 번만 닫는다.
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

/// `TerminateProcess` 호출 하나. ★실패도 힙을 쓰지 않는 오류로 만든다★ — 자물쇠 안에서 불리므로
/// `win_err`(`io::Error::other` = 상자 할당)를 쓰지 않는다. windows 0.58 의 `ok()` 는 `GetLastError` 를 읽기만 해
/// 마지막 오류가 그대로 남아 있다.
// ADR-0257
fn terminate_handle(handle: HANDLE, exit_code: u32) -> io::Result<()> {
    // SAFETY: 호출자가 쥔 유효 프로세스 핸들 — 끝내기 권한이 없으면 OS 가 거절한다.
    unsafe { TerminateProcess(handle, exit_code) }.map_err(|_| io::Error::last_os_error())
}

/// 끝내기 날것 결과 가르기 — 실패여도 그 사이 스스로 끝났으면 `Gone`.
// ADR-0257
fn classify_terminate(handle: HANDLE, raw: io::Result<()>) -> io::Result<MemberOutcome> {
    match raw {
        Ok(()) => Ok(MemberOutcome::Terminated),
        // SAFETY: 호출자가 쥔 유효 핸들 · 기다리지 않는 조회.
        Err(_) if unsafe { WaitForSingleObject(handle, 0) } == WAIT_OBJECT_0 => {
            Ok(MemberOutcome::Gone)
        }
        Err(e) => Err(e),
    }
}

/// 대기 ms — `u32::MAX` 는 INFINITE 라 그 아래로 자른다.
fn finite_millis(wait: Duration) -> u32 {
    u32::try_from(wait.as_millis())
        .unwrap_or(u32::MAX)
        .min(u32::MAX - 1)
}

/// 핸들이 `millis` 안에 신호를 받았나(= 끝났나). 생존을 종료 코드(STILL_ACTIVE = 259)로 재지 않는다 — 259 로
/// 끝난 프로세스가 산 것으로 보인다.
fn signaled_within(handle: HANDLE, millis: u32) -> io::Result<bool> {
    // SAFETY: 호출자가 쥔 유효 핸들 · 유한 대기.
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
    // SAFETY: 유효 핸들 + 그 클래스의 배치대로 잡은 스택 구조체와 정확한 크기 + 스택 출력.
    let status = unsafe {
        NtQueryInformationProcess(
            handle,
            ProcessBasicInformation,
            (&mut info as *mut PROCESS_BASIC_INFORMATION).cast(),
            size,
            &mut written,
        )
    };
    // 돌려받은 크기까지 맞아야 배치를 믿는다.
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
    // SAFETY: 유효 핸들 + 스택의 4개 FILETIME 출력 포인터.
    unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) }.ok()?;
    Some(((creation.dwHighDateTime as u64) << 32) | (creation.dwLowDateTime as u64))
}

fn image_path(handle: HANDLE) -> Option<String> {
    let mut buf = vec![0u16; IMAGE_PATH_CHARS];
    let mut len = IMAGE_PATH_CHARS as u32;
    // SAFETY: 유효 핸들 + 길이를 알려 준 쓰기 가능 버퍼 + 스택 입출력 길이.
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

/// `QUERY_LIMITED` 권한으로 읽힌다는 것은 문서가 아니라 실측이다(TRD t40 §8 ㉒ⓘ) — 거절되면 빈 칸이 된다.
fn command_line(handle: HANDLE) -> Option<String> {
    const HEAD: usize = core::mem::size_of::<UNICODE_STRING>();
    // u64 칸으로 잡아 머리의 포인터 칸 정렬을 맞춘다.
    let mut buf = vec![0u64; COMMAND_LINE_BYTES.div_ceil(8)];
    let bytes = buf.len() * 8;
    let mut written = 0u32;
    // SAFETY: 유효 핸들 + 쓰기 가능한 `bytes` 바이트 버퍼(8 정렬) + 스택 출력.
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
    // SAFETY: 버퍼는 머리 크기 이상이고 8 정렬이다. `UNICODE_STRING` 은 Copy 라 값으로 읽는다.
    let head = unsafe { buf.as_ptr().cast::<UNICODE_STRING>().read() };
    let units = usize::from(head.Length) / 2;
    if units == 0 {
        return Some(String::new());
    }
    // 글자는 커널이 우리 버퍼 안 머리 뒤에 채우고 Buffer 가 그 자리를 가리킨다 — 그 주소를 그대로 믿지 않고
    //   우리 버퍼 안의 오프셋으로 바꿔 범위를 확인한 뒤 우리 버퍼에서 읽는다.
    let offset = (head.Buffer.0 as usize).checked_sub(buf.as_ptr() as usize)?;
    if offset < HEAD || !offset.is_multiple_of(2) || offset.checked_add(units * 2)? > bytes {
        return None;
    }
    // SAFETY: [offset, offset + units * 2) 는 위에서 확인한 버퍼 안 범위이고 2 정렬이다(버퍼가 8 정렬).
    let text = unsafe {
        core::slice::from_raw_parts(buf.as_ptr().cast::<u8>().add(offset).cast::<u16>(), units)
    };
    Some(String::from_utf16_lossy(text))
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

// 실프로세스 시험 — 시험마다 넷 이하(콘솔 호스트 포함): 몰아 띄우면 개발 PC 터미널이 죽는다(CLAUDE.md). 실프로세스
// 도우미는 중립 손잡이의 시험도 가져다 쓴다 — 이 파일이 그쪽을 부르지 않게 도우미를 여기 둔다.
#[cfg(test)]
pub(crate) mod tests {
    use std::io::Write;
    use std::process::{Child, Command, Stdio};
    use std::time::Instant;

    use engram_dashboard_base::platform::{process_start, ProcessStart};

    use super::*;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const DETACHED_PROCESS: u32 = 0x0000_0008;

    /// `cmd.exe /d /c <line>` — 단 stdin 에서 한 줄을 받을 때까지 `line` 을 시작하지 않는다. 그 사이에 Job 에
    /// 넣으면 `line` 이 띄우는 것은 전부 Job 안에서 태어난다. [`open_gate`] 가 그 한 줄을 준다.
    pub(crate) fn spawn_gated_cmd(line: &str, flags: u32) -> Child {
        use std::os::windows::process::CommandExt;
        Command::new("cmd.exe")
            .raw_arg(format!("/d /c set /p _= & {line}"))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(flags)
            .spawn()
            .expect("cmd.exe 기동")
    }

    pub(crate) fn open_gate(child: &mut Child) {
        let mut stdin = child.stdin.take().expect("stdin 파이프");
        let _ = stdin.write_all(b"go\r\n");
    }

    pub(crate) fn wait_until<T>(what: &str, mut probe: impl FnMut() -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(found) = probe() {
                return found;
            }
            assert!(Instant::now() < deadline, "10 초 안에 {what} 를 못 봤다");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// 그 번호의 실행 파일이 `ping.exe` 인가. 못 열면 `false`.
    pub(crate) fn is_ping(pid: u32) -> bool {
        // SAFETY: 조회 권한만 연다.
        let Ok(handle) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) })
        else {
            return false;
        };
        let image = image_path(handle);
        // SAFETY: 방금 연 핸들을 한 번 닫는다.
        unsafe {
            let _ = CloseHandle(handle);
        }
        image.is_some_and(|p| p.to_ascii_lowercase().ends_with("\\ping.exe"))
    }

    /// `want` 에 맞는 알림이 올 때까지 꺼낸다(10 초) — 그 앞의 알림은 버린다.
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

    /// 쌓인 알림을 `Timeout` 이 날 때까지 비운다.
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

    /// ⑥ 붙든 핸들로 읽은 사실 — 부모 = 띄운 이 시험 프로세스 · 생성은 이 시험 프로세스보다 늦다 · 실행 파일 ·
    /// 명령줄(조회 권한만으로). 조회 권한만으로 붙든 것은 끝낼 수 없다.
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

        let mut x = spawn_gated_cmd("rem t40-facts", CREATE_NO_WINDOW);
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

        let refused = watcher.terminate_raw();
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
        let raw = killer.terminate_raw();
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
        assert_eq!(status.code(), Some(LEFTOVER_EXIT_CODE as i32));
    }

    /// ⑦-가 `start` 가 실패하면 Job 은 포트에 붙지 않는다 — 원래 멤버의 되알림도, 그 뒤 Job 안에서 태어난 것도
    /// 오지 않는다(붙었다면 둘 다 이미 쌓여 있다 — ⑦-나).
    #[test]
    fn a_failed_start_leaves_the_job_unwatched() {
        let job = JobObjectHandle::new().expect("Job 생성");
        let mut x = spawn_gated_cmd("ping -n 30 127.0.0.1", CREATE_NO_WINDOW);
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

    /// ⑦-나 붙은 포트 — 붙일 때 이미 든 X 가 다시 알려지고 · X 가 띄운 ping 이 `Joined` · 그 ping 이 끝나면
    /// `Other` · 뗀 뒤 태어난 멤버는 알려지지 않는다.
    #[test]
    fn a_watched_job_reports_births_until_unwatched() {
        let job = JobObjectHandle::new().expect("Job 생성");
        // 문이 둘이다 — 첫 줄에 첫 ping, 둘째 줄에 둘째 ping. 둘째 문은 뗀 뒤에 연다.
        let mut x = spawn_gated_cmd(
            "ping -n 30 127.0.0.1 & set /p _= & ping -n 30 127.0.0.1",
            CREATE_NO_WINDOW,
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
        // 끝까지 쥔다 — 둘째 ping 이 이 번호를 재사용하지 못하게.
        let ping = job
            .pin_member(first, true)
            .expect("붙들기")
            .expect("우리 멤버");
        assert_eq!(ping.facts().ppid, root, "ping 의 부모: {:?}", ping.facts());
        // 끝내기 전에 비운다 — 아래 `Other` 는 끝내기 뒤에 온 것이 되고, 그때 남은 멤버는 기다리는 X 뿐이라 ping 의
        // 끝남 말고 올 자리가 없다.
        drain(&port);
        let raw = ping.terminate_raw();
        assert_eq!(
            ping.classify(raw).expect("끝내기"),
            MemberOutcome::Terminated
        );
        next_matching(&port, "ping 의 끝남", |e| e == PortOutcome::Other);
        // 곧바로 `exited()` 로 묻지 않는다 — 끝남 알림이 프로세스 객체의 신호보다 먼저 설 수 있다(미실측).
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

    /// ⑧ MSYS 명령줄의 세 모양 — 훅 사본 판정이 기댄다(TRD t40 §3-0): 껍데기가 exec 한 스크립트는 제 명령줄을
    /// 갖고 · 그 서브셸(fork)은 부모와 실행 파일 · 명령줄이 같고 · 스크립트가 exec 한 `sleep` 은 다르다.
    /// 껍데기는 문(`read _`)을 지난 뒤 명령 하나를 exec 한다 — 훅의 `-c "bash x.sh"` 처럼 fork 없이 넘기면서
    /// 모두 Job 안에서 태어나게. 콘솔 없이 띄워 콘솔 호스트를 끌고 오지 않는다(껍데기 · 스크립트 · 서브셸 ·
    /// `sleep` 넷).
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
        // 비대화 셸은 백그라운드 명령의 stdin 을 /dev/null 로 돌린다 — 서브셸이 파이프를 fd 3 으로 받아 기다려야
        // 끝까지 산다.
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
        // `/usr/bin` 을 절대 경로로 부른다 — Git 런처를 거치지 않은 `usr\bin\bash.exe` 는 `PATH` 에 `/usr/bin` 이
        // 없다(실측).
        let mut shell = Command::new(BASH)
            .args(["-c", &format!("read _; exec /usr/bin/bash {script_arg}")])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(DETACHED_PROCESS)
            .spawn()
            .expect("bash 기동");
        let root = shell.id();
        job.assign(root).expect("Job 편입");
        // 서브셸의 `read _` 가 이 파이프에서 기다린다 — 끝까지 쥔다.
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

        // 태어나는 대로 붙든다 — 짧게 사는 fork 를 끝나기 전에 연다.
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

        // 포트를 놓기 전에 뗀다(BirthPort 의 규칙).
        job.unwatch_births().expect("떼기");
        job.terminate(1).expect("Job 끝내기");
        drop(gate);
        let _ = shell.wait();
        let _ = std::fs::remove_file(&script);
    }
}
