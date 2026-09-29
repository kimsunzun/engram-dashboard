//! 통로가 띄운 프로세스 무리의 **중립 손잡이** — 멤버 명단 · 멤버 하나의 검증 · 검증된 것 하나 끝내기.
//!
//! 무엇을 끝낼지는 모른다(ADR-0004) — 고르기는 이것을 받아 쓰는 쪽의 몫이고, 여기는 받은 신원이 아직 우리
//! 무리의 산 멤버인지 OS 에 묻고 그 핸들로 끝낼 뿐이다.
//!
//! ★OS 갈래는 이 모듈 안에서만 선다(ADR-0230)★ — Windows 는 Job Object 조각을 감싸고, 그 밖의 OS 는 멤버가
//! 없고 누구든 `Gone` 인 껍데기다. 부르는 쪽에는 `cfg!(windows)` 가 없다.
//!
//! ★Job 을 약하게 쥔다★ — 강하게 쥐면 통로가 사라진 뒤에도 Job 핸들이 안 닫혀 `KILL_ON_JOB_CLOSE` 가 늦어진다.
//! 부를 때마다 잠깐 올리고, 통로가 사라졌으면 명단은 비고 검증은 `Gone` 이다.
// ADR-0257

use std::io;
#[cfg(windows)]
use std::sync::Weak;

use super::process_tree::ProcessIdentity;
#[cfg(windows)]
use super::windows::{
    JobObjectHandle, MemberCheck, MemberOutcome, VerifiedMember, LEFTOVER_EXIT_CODE,
};

/// [`ProcessGroup::verify`] 의 답. `Ready` 만 끝내기로 간다.
///
/// - `Gone` = 그 번호의 프로세스가 없거나 이미 끝났다 · 무리가 이미 사라졌다.
/// - `NotOurs` = 산 프로세스지만 우리 무리 밖이거나 시작시각이 달라 그 신원이 아니다.
pub(crate) enum Verify {
    Ready(Box<dyn ReadyKill>),
    Gone,
    NotOurs,
}

/// 검증을 마친 멤버 하나 — 쥐고 있는 동안 그 PID 는 남에게 넘어가지 않는다.
///
/// 두 단계로 갈린 것은 부르는 쪽이 자물쇠 안에서 OS 호출을 **하나만** 하게 하려는 것이다: `terminate_raw` =
/// 끝내기 호출 하나 · `settle` = 그 결과 가르기와 핸들 닫기. `settle` 없이 버려도 핸들은 닫힌다.
pub(crate) trait ReadyKill: Send {
    fn terminate_raw(&self) -> io::Result<()>;
    fn settle(self: Box<Self>, raw: io::Result<()>) -> io::Result<MemberKill>;
}

/// `Gone` = 검증 뒤 끝내기 전에 스스로 끝났다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MemberKill {
    Terminated,
    Gone,
}

pub(crate) struct ProcessGroup {
    #[cfg(windows)]
    job: Weak<JobObjectHandle>,
    root_attached: usize,
}

impl ProcessGroup {
    #[cfg(windows)]
    pub(crate) fn new(job: Weak<JobObjectHandle>, root_attached: usize) -> Self {
        Self { job, root_attached }
    }

    /// 무리가 이미 사라진 손잡이 — 모든 OS 에서 같은 모양이다.
    #[cfg(test)]
    pub(crate) fn detached(root_attached: usize) -> Self {
        Self {
            #[cfg(windows)]
            job: Weak::new(),
            root_attached,
        }
    }

    /// 완전한 멤버 PID 명단(뿌리 포함). 통로가 사라졌으면 `Ok(빈 목록)` · 불완전하면 `Err`.
    pub(crate) fn member_pids(&self) -> io::Result<Vec<u32>> {
        #[cfg(windows)]
        {
            match self.job.upgrade() {
                Some(job) => job.member_pids(),
                None => Ok(Vec::new()),
            }
        }
        #[cfg(not(windows))]
        {
            Ok(Vec::new())
        }
    }

    /// 그 신원이 아직 우리 무리의 산 멤버인지 본다. 끝내지 않는다. `Err` = 판정에 드는 OS 조회가 실패했다.
    pub(crate) fn verify(&self, who: ProcessIdentity) -> io::Result<Verify> {
        #[cfg(windows)]
        {
            let Some(job) = self.job.upgrade() else {
                return Ok(Verify::Gone);
            };
            Ok(match job.verify_member(who.pid, who.start_time)? {
                MemberCheck::Ready(member) => Verify::Ready(Box::new(JobMember(member))),
                MemberCheck::Gone => Verify::Gone,
                MemberCheck::NotOurs => Verify::NotOurs,
            })
        }
        #[cfg(not(windows))]
        {
            let _ = who;
            Ok(Verify::Gone)
        }
    }

    /// 이 스폰이 뿌리 아래 붙이는 프로세스 수 — 프로그램이 아니라 스폰 방식이 만드는 것(예: 콘솔 호스트).
    pub(crate) fn root_attached(&self) -> usize {
        self.root_attached
    }
}

#[cfg(windows)]
struct JobMember(VerifiedMember);

#[cfg(windows)]
impl ReadyKill for JobMember {
    fn terminate_raw(&self) -> io::Result<()> {
        self.0.terminate_raw(LEFTOVER_EXIT_CODE)
    }

    fn settle(self: Box<Self>, raw: io::Result<()>) -> io::Result<MemberKill> {
        Ok(match self.0.settle(raw)? {
            MemberOutcome::Terminated => MemberKill::Terminated,
            MemberOutcome::Gone => MemberKill::Gone,
        })
    }
}

// 실프로세스 도우미는 `backend::claude` 의 고르기 실측 시험도 쓴다 — 그 시험을 여기 두면 이 층이 claude 를 부른다.
#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// 통로가 사라진 손잡이 · 비Windows 껍데기가 같은 답을 준다 — 멤버 없음 · 누구든 `Gone`.
    #[test]
    fn a_group_without_a_live_job_lists_nothing_and_owns_nobody() {
        let group = ProcessGroup::detached(1);
        assert_eq!(group.member_pids().expect("명단"), Vec::<u32>::new());
        let me = ProcessIdentity {
            pid: std::process::id(),
            start_time: 1,
        };
        assert!(matches!(group.verify(me), Ok(Verify::Gone)));
        assert_eq!(group.root_attached(), 1);
    }

    // ── 실프로세스(Windows) — 시험마다 몇 개만 차례로 띄운다: 몰아 띄우면 개발 PC 터미널이 죽는다(CLAUDE.md) ──

    #[cfg(windows)]
    use std::io::Write;
    #[cfg(windows)]
    use std::process::{Child, Command, Stdio};
    #[cfg(windows)]
    use std::sync::Arc;
    #[cfg(windows)]
    use std::time::{Duration, Instant};

    #[cfg(windows)]
    use engram_dashboard_base::platform::{process_parent_table, process_start, ProcessStart};

    #[cfg(windows)]
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    #[cfg(windows)]
    const DETACHED_PROCESS: u32 = 0x0000_0008;

    #[cfg(windows)]
    pub(crate) fn new_group() -> (Arc<JobObjectHandle>, ProcessGroup) {
        let job = Arc::new(JobObjectHandle::new().expect("Job 생성"));
        let group = ProcessGroup::new(Arc::downgrade(&job), 1);
        (job, group)
    }

    /// `cmd.exe /d /c <line>` — 단 stdin 에서 한 줄을 받을 때까지 `line` 을 시작하지 않는다. 그 사이에 Job 에
    /// 넣으면 `line` 이 띄우는 것은 전부 Job 안에서 태어난다. [`open_gate`] 가 그 한 줄을 준다.
    #[cfg(windows)]
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

    #[cfg(windows)]
    pub(crate) fn open_gate(child: &mut Child) {
        let mut stdin = child.stdin.take().expect("stdin 파이프");
        let _ = stdin.write_all(b"go\r\n");
    }

    /// 콘솔 없이 도는 `ping` 하나 — 콘솔 호스트를 끌고 오지 않는다.
    #[cfg(windows)]
    fn spawn_lone_ping() -> Child {
        use std::os::windows::process::CommandExt;
        Command::new("ping.exe")
            .args(["-n", "30", "127.0.0.1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(DETACHED_PROCESS)
            .spawn()
            .expect("ping.exe 기동")
    }

    #[cfg(windows)]
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

    #[cfg(windows)]
    pub(crate) fn identity(pid: u32) -> ProcessIdentity {
        match process_start(pid) {
            ProcessStart::Known(start_time) => ProcessIdentity { pid, start_time },
            other => panic!("PID {pid} 의 시작시각을 못 읽었다: {other:?}"),
        }
    }

    /// 실행 파일 경로(소문자). 못 열면 `None`.
    #[cfg(windows)]
    fn image_path(pid: u32) -> Option<String> {
        use windows::core::PWSTR;
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
            PROCESS_QUERY_LIMITED_INFORMATION,
        };
        // SAFETY: 조회 권한만 연다.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        // SAFETY: 유효 핸들 + 길이를 알려 준 스택 버퍼.
        let asked = unsafe {
            QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                PWSTR(buf.as_mut_ptr()),
                &mut len,
            )
        };
        // SAFETY: 방금 연 핸들을 한 번 닫는다.
        unsafe {
            let _ = CloseHandle(handle);
        }
        asked.ok()?;
        Some(String::from_utf16_lossy(&buf[..len as usize]).to_ascii_lowercase())
    }

    #[cfg(windows)]
    pub(crate) fn is_ping(pid: u32) -> bool {
        image_path(pid).is_some_and(|p| p.ends_with("\\ping.exe"))
    }

    /// 표의 자기 줄에서 부모 PID.
    #[cfg(windows)]
    pub(crate) fn parent_row(table: &[(u32, u32)], pid: u32) -> Option<u32> {
        table.iter().find(|(p, _)| *p == pid).map(|(_, ppid)| *ppid)
    }

    #[cfg(windows)]
    fn ppid_of(table: &[(u32, u32)], pid: u32) -> u32 {
        parent_row(table, pid).unwrap_or_else(|| panic!("PID {pid} 의 줄이 표에 없다"))
    }

    /// TRD t40 §3-4 의 「부모 죽음」을 시험용으로 옮긴 사본 — 부모가 표에 없다 · 사라졌다 · 그 번호를 자기보다
    /// 늦게 태어난 것이 쥐었다.
    #[cfg(windows)]
    fn parent_is_dead(table: &[(u32, u32)], member: ProcessIdentity) -> bool {
        let ppid = ppid_of(table, member.pid);
        if !table.iter().any(|(p, _)| *p == ppid) {
            return true;
        }
        match process_start(ppid) {
            ProcessStart::Gone => true,
            ProcessStart::Known(parent_start) => parent_start > member.start_time,
            ProcessStart::Unknown => false,
        }
    }

    /// 프로세스 하나를 지켜보는 핸들 — 끝나기를 기다리고 종료 코드를 읽는다. 쥐는 동안 그 프로세스 객체가 남는다.
    #[cfg(windows)]
    struct Watch(windows::Win32::Foundation::HANDLE);

    #[cfg(windows)]
    impl Watch {
        fn open(pid: u32) -> Self {
            use windows::Win32::System::Threading::{
                OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
            };
            // SAFETY: 조회 · 대기 권한만 연다.
            let handle = unsafe {
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                    false,
                    pid,
                )
            }
            .expect("지켜볼 핸들");
            Self(handle)
        }

        fn exit_code_within(&self, limit: Duration) -> Option<u32> {
            use windows::Win32::Foundation::WAIT_OBJECT_0;
            use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
            // SAFETY: 유효 핸들 · 유한 대기.
            if unsafe { WaitForSingleObject(self.0, limit.as_millis() as u32) } != WAIT_OBJECT_0 {
                return None;
            }
            let mut code = 0u32;
            // SAFETY: 유효 핸들 + 스택 출력.
            unsafe { GetExitCodeProcess(self.0, &mut code) }.ok()?;
            Some(code)
        }

        fn alive(&self) -> bool {
            use windows::Win32::Foundation::WAIT_TIMEOUT;
            use windows::Win32::System::Threading::WaitForSingleObject;
            // SAFETY: 유효 핸들 · 기다리지 않는 조회.
            let state = unsafe { WaitForSingleObject(self.0, 0) };
            state == WAIT_TIMEOUT
        }
    }

    #[cfg(windows)]
    impl Drop for Watch {
        fn drop(&mut self) {
            // SAFETY: open 이 연 핸들을 한 번 닫는다.
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(self.0);
            }
        }
    }

    /// ① Job 안 cmd 의 자식 ping 을 검증하고 끝낸다 — 종료 코드로 누가 끝냈는지 남는다.
    #[cfg(windows)]
    #[test]
    fn a_verified_member_is_terminated_with_the_leftover_code() {
        let (job, group) = new_group();
        let mut root = spawn_gated_cmd("ping -n 30 127.0.0.1", CREATE_NO_WINDOW);
        job.assign(root.id()).expect("Job 편입");
        open_gate(&mut root);

        let ping = wait_until("Job 안의 ping", || {
            group
                .member_pids()
                .ok()?
                .into_iter()
                .find(|&pid| is_ping(pid))
        });
        let who = identity(ping);
        let watch = Watch::open(ping);

        let Verify::Ready(ready) = group.verify(who).expect("검증") else {
            panic!("산 멤버가 Ready 가 아니다");
        };
        let raw = ready.terminate_raw();
        assert_eq!(ready.settle(raw).expect("끝내기"), MemberKill::Terminated);
        assert_eq!(
            watch.exit_code_within(Duration::from_secs(5)),
            Some(LEFTOVER_EXIT_CODE)
        );
        drop(watch);
        wait_until("명단에서 빠진 ping", || {
            (!group.member_pids().ok()?.contains(&ping)).then_some(())
        });
        let _ = root.wait();
    }

    /// ③ 우리 Job 의 산 멤버라도 시작시각이 다르면 다른 프로세스다 — 끝내기로 가지 않고 그 프로세스는 산다.
    #[cfg(windows)]
    #[test]
    fn a_member_with_another_start_time_is_not_ours_and_survives() {
        let (job, group) = new_group();
        let mut ping = spawn_lone_ping();
        job.assign(ping.id()).expect("Job 편입");
        let who = identity(ping.id());

        let stale = ProcessIdentity {
            pid: who.pid,
            start_time: who.start_time.wrapping_add(1),
        };
        assert!(matches!(group.verify(stale), Ok(Verify::NotOurs)));
        assert!(ping.try_wait().expect("상태").is_none(), "검증이 죽였다");
        assert!(
            matches!(group.verify(who), Ok(Verify::Ready(_))),
            "맞는 신원은 Ready 여야 앞 단언이 뜻이 있다"
        );

        let _ = ping.kill();
        let _ = ping.wait();
    }

    /// ④ 다른 Job 의 프로세스는 우리 것이 아니다(조사 §4 사고의 회귀망).
    #[cfg(windows)]
    #[test]
    fn a_process_in_another_job_is_not_ours() {
        let (_ours, group) = new_group();
        let other = JobObjectHandle::new().expect("남의 Job");
        let mut stranger = spawn_lone_ping();
        other.assign(stranger.id()).expect("남의 Job 편입");

        assert!(matches!(
            group.verify(identity(stranger.id())),
            Ok(Verify::NotOurs)
        ));
        assert!(stranger.try_wait().expect("상태").is_none(), "남이 죽었다");

        let _ = stranger.kill();
        let _ = stranger.wait();
    }

    /// ④ 중첩 변형 — 우리 Job 이 부모 Job P 아래로 중첩돼도 P 에만 든 남은 우리 것이 아니다. Job 인자로 NULL 을
    /// 넘기면(「아무 Job 에나 들었나」) 여기서 빨개진다.
    #[cfg(windows)]
    #[test]
    fn a_process_only_in_the_enclosing_job_is_not_ours() {
        let parent = JobObjectHandle::new().expect("부모 Job");
        let (ours, group) = new_group();
        let mut member = spawn_lone_ping();
        parent.assign(member.id()).expect("부모 Job 편입");
        ours.assign(member.id())
            .expect("우리 Job 편입 — 부모 아래로 중첩");
        let mut stranger = spawn_lone_ping();
        parent.assign(stranger.id()).expect("부모 Job 에만 편입");

        assert!(
            matches!(group.verify(identity(member.id())), Ok(Verify::Ready(_))),
            "중첩이 안 섰다 — 우리 멤버가 Ready 가 아니다"
        );
        assert!(matches!(
            group.verify(identity(stranger.id())),
            Ok(Verify::NotOurs)
        ));
        assert!(stranger.try_wait().expect("상태").is_none(), "남이 죽었다");

        for child in [&mut member, &mut stranger] {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// ⑤ 끝났지만 핸들이 붙든(좀비) 부모는 표에 없어야 한다 — 표가 좀비를 싣는다면 죽은 부모가 산 것으로 읽혀
    /// 잔여물을 놓친다. Job 명단에서도 빠지고 명단은 완전하다 — 좀비가 멤버 수에만 남으면 명단이 영영 불완전해
    /// 정리가 한 번도 돌지 못한다.
    #[cfg(windows)]
    #[test]
    fn a_dead_parent_held_open_is_not_in_the_table() {
        let (job, group) = new_group();
        let mut parent = spawn_gated_cmd("start \"\" /b ping -n 30 127.0.0.1", 0);
        let parent_pid = parent.id();
        job.assign(parent_pid).expect("Job 편입");
        open_gate(&mut parent);
        let child = wait_until("P 가 띄운 ping", || {
            let table = process_parent_table()?;
            group
                .member_pids()
                .ok()?
                .into_iter()
                .find(|&pid| is_ping(pid) && parent_row(&table, pid) == Some(parent_pid))
        });
        let _ = parent.kill();
        let _ = parent.wait();
        // `parent` 를 아직 쥐고 있다 — 그 핸들이 P 의 프로세스 객체를 붙든다.

        let members = group
            .member_pids()
            .expect("끝났지만 핸들이 붙든 멤버가 있어도 명단은 완전해야 한다");
        assert!(
            !members.contains(&parent_pid),
            "끝났지만 핸들이 붙든 P 가 명단에 남았다: {members:?}"
        );
        assert!(members.contains(&child), "산 C 가 명단에 없다: {members:?}");

        let table = process_parent_table().expect("표");
        assert!(
            !table.iter().any(|(pid, _)| *pid == parent_pid),
            "끝났지만 핸들이 붙든 P 가 표에 있다 — 표가 좀비를 싣는다"
        );
        assert!(
            parent_is_dead(&table, identity(child)),
            "C 의 부모가 죽음으로 안 읽힌다"
        );
        drop(parent);
    }

    /// ⑥ 첫 칸 하나로 물어도 늘려 되물어 전부 나온다.
    #[cfg(windows)]
    #[test]
    fn a_one_slot_first_ask_still_lists_every_member() {
        let (job, _group) = new_group();
        let mut root = spawn_gated_cmd("ping -n 30 127.0.0.1", CREATE_NO_WINDOW);
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

    /// ⑦ 부모가 끝난 MSYS fork 자식이 우리 Job 멤버로 남는지 · 부모 죽음으로 읽히는지 · 끝나는지.
    /// ★Job 상속만 잰다 — 멈춤은 재현하지 않는다★. `PATH` 의 `bash` 는 WSL 일 수 있어 경로를 박는다.
    #[cfg(windows)]
    #[test]
    #[ignore = "Git Bash(C:\\Program Files\\Git\\usr\\bin\\bash.exe) 가 있어야 한다"]
    fn an_orphaned_msys_fork_child_stays_in_our_job_and_ends() {
        use std::os::windows::process::CommandExt;
        const BASH: &str = r"C:\Program Files\Git\usr\bin\bash.exe";
        assert!(
            std::path::Path::new(BASH).exists(),
            "{BASH} 가 없다 — PATH 의 bash 로 대신하지 않는다"
        );
        let (job, group) = new_group();
        // `sleep` 을 절대 경로로 부른다 — Git 런처(`Git\bin\bash.exe`)를 거치지 않은 `usr\bin\bash.exe` 는 `PATH` 에
        // `/usr/bin` 이 없어 맨 `sleep` 이 127(없는 명령)로 끝난다(실측).
        let mut bash = Command::new(BASH)
            .args(["-c", "read _; (/usr/bin/sleep 30 &)"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .expect("bash 기동");
        job.assign(bash.id()).expect("Job 편입");
        open_gate(&mut bash);
        let status = bash.wait().expect("bash 종료");
        assert!(
            status.success(),
            "bash 가 {status:?} 로 끝났다 — 스크립트가 안 돌았다"
        );
        drop(bash);

        let orphans = wait_until("부모가 끝난 MSYS 프로세스", || {
            let table = process_parent_table()?;
            let found: Vec<ProcessIdentity> = group
                .member_pids()
                .ok()?
                .into_iter()
                .filter(|&pid| image_path(pid).is_some_and(|p| p.contains("\\usr\\bin\\")))
                .filter(|&pid| parent_row(&table, pid).is_some())
                .filter_map(|pid| match process_start(pid) {
                    ProcessStart::Known(start_time) => Some(ProcessIdentity { pid, start_time }),
                    _ => None,
                })
                .filter(|m| parent_is_dead(&table, *m))
                .collect();
            (!found.is_empty()).then_some(found)
        });
        for orphan in orphans {
            let watch = Watch::open(orphan.pid);
            let Verify::Ready(ready) = group.verify(orphan).expect("검증") else {
                panic!("부모가 끝난 멤버 {} 가 Ready 가 아니다", orphan.pid);
            };
            let raw = ready.terminate_raw();
            assert_eq!(ready.settle(raw).expect("끝내기"), MemberKill::Terminated);
            assert_eq!(
                watch.exit_code_within(Duration::from_secs(5)),
                Some(LEFTOVER_EXIT_CODE)
            );
            assert!(!watch.alive());
        }
    }
}
