//! 통로가 띄운 프로세스 무리의 **중립 손잡이** — 멤버 명단 · 멤버 하나 붙들기(사실 · 끝내기) · 가입 알림 포트 ·
//! 통로의 물러남 표시(읽기 전용).
//!
//! 무엇을 끝낼지는 모른다(ADR-0004) — 고르기는 이것을 받아 쓰는 쪽의 몫이고, 여기는 받은 번호가 우리 무리의
//! 멤버인지 OS 에 물어 붙들고 그 핸들로 사실을 읽고 끝낼 뿐이다.
//!
//! ★OS 갈래는 이 모듈 안에서만 선다(ADR-0230)★ — Windows 는 Job Object 조각을 감싸고, 그 밖의 OS 는 멤버가
//! 없고 아무도 못 붙들며 포트가 `Unsupported` 인 껍데기다. 부르는 쪽에는 `cfg!(windows)` 가 없다. OS 와 무관한
//! 값 타입([`ProcessFacts`] · [`PortEvent`])은 여기 두고, Windows 조각은 제 모양을 따로 둬 여기서 옮겨 싣는다 —
//! 그 조각은 이 모듈을 부르지 않는다(그 파일 헤더).
//!
//! ★Job 을 약하게 쥔다★ — 강하게 쥐면 통로가 사라진 뒤에도 Job 핸들이 안 닫혀 `KILL_ON_JOB_CLOSE` 가 늦어진다.
//! 부를 때마다 그 호출 동안만 올린다 — 가장 긴 창은 [`ProcessGroup::watch_births`] 가 `start` 를 부르는 동안이다
//! (그래서 `start` 는 막히지 않아야 한다). 통로가 사라졌으면 명단은 비고 아무도 못 붙들며 포트는 [`GROUP_GONE`]
//! 이다. 내준 붙든 멤버 · 포트 · 표시는 Job 을 붙들지 않는다.
// ADR-0257

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
#[cfg(windows)]
use std::sync::Weak;
use std::time::Duration;

#[cfg(windows)]
use super::windows::{
    BirthPort, JobObjectHandle, MemberFacts, MemberOutcome, PinnedMember, PortOutcome,
};

/// 붙든 핸들로 **한 번** 읽은 사실 — 번호가 아니라 그 프로세스 객체의 것이다. 못 읽은 칸은 `ppid` · `create` =
/// 0 · 문자열 = 빈 문자열이다.
///
/// - `ppid` = 만들 때 적힌 부모 번호 — 그 번호가 지금도 같은 부모라는 보장은 없다.
/// - `create` = 생성 시각(FILETIME · 100 ns) — 두 프로세스의 생성 순서를 비교하는 데만 뜻이 있다.
/// - `image` = 실행 파일 전체 경로(Win32 형식) · `cmdline` = 명령줄 그대로.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ProcessFacts {
    pub ppid: u32,
    pub create: u64,
    pub image: String,
    pub cmdline: String,
}

/// [`Births::next`] 의 답.
///
/// - `Joined(pid)` = 멤버가 새로 들었다. 번호뿐이라 꺼낸 때 이미 끝나 재사용됐을 수 있다 — 곧바로 붙들어야
///   신원이 선다.
/// - `Other` = 그 밖의 알림(끝남 등) 또는 이 포트의 것이 아닌 패킷.
/// - `Timeout` = 기다리는 동안 알림이 없었다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PortEvent {
    Joined(u32),
    Other,
    Timeout,
}

/// 통로의 물러남 표시를 **읽기만** 하는 사본 — 칸의 주인(통로)이 [`Self::of`] 로 만들어 [`ProcessGroup`] 에 싣는다.
/// ★세우는 메서드가 없다★ — 세우는 것은 칸의 주인뿐이고, 읽는 쪽은 manager 도 통로의 문도 모른 채 이것만 본다.
#[derive(Debug, Clone)]
pub(crate) struct RetiringSignal(Arc<AtomicBool>);

impl RetiringSignal {
    /// 칸을 나눠 쥔다 — 칸의 주인만 부른다. 주인은 `Release` 로 세운다.
    pub(crate) fn of(flag: &Arc<AtomicBool>) -> Self {
        Self(Arc::clone(flag))
    }

    pub(crate) fn is_set(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// 우리 무리의 멤버로 확인하고 붙든 프로세스 하나 — 쥐는 동안 그 번호는 재사용되지 않고, 끝난 뒤에도 사실은
/// 남는다. 무리(Job)를 붙들지 않는다. 버리면 핸들이 닫힌다.
///
/// 끝내기가 두 메서드로 갈린 것은 부르는 쪽이 자물쇠 안에서 OS 호출을 **하나만** 하게 하려는 것이다:
/// `terminate_raw` = 끝내기 호출 하나 · `classify` = 그 날것 결과 가르기.
pub(crate) trait Pinned: Send + Sync {
    /// 지금 끝났나 — 기다리지 않는다. `Err` = 대기 조회 자체의 실패.
    fn exited(&self) -> io::Result<bool>;
    /// `d` 까지 끝나기를 기다린다 — 끝났으면 `true`. `Err` = 대기 조회 자체의 실패.
    fn wait_exit(&self, d: Duration) -> io::Result<bool>;
    fn facts(&self) -> &ProcessFacts;
    /// 끝내기 호출 하나 — 대상이 끝나기를 기다리지 않는다. 끝내기 권한 없이 붙든 것은 늘 `Err` 다.
    fn terminate_raw(&self) -> io::Result<()>;
    /// [`Self::terminate_raw`] 의 날것 결과를 가른다 — 실패여도 그 사이 스스로 끝났으면 `Gone` 이다.
    fn classify(&self, raw: io::Result<()>) -> io::Result<MemberKill>;
}

/// 무리의 가입 알림 포트 — [`ProcessGroup::watch_births`] 가 `start` 에 넘긴다. 여러 스레드가 나눠 쥐고, 마지막
/// 것이 사라지면 포트 핸들이 닫힌다. 무리(Job)를 붙들지 않는다.
///
/// 알림은 통지일 뿐 배달이 보장되지 않는다(MS Learn) — 빠진 가입은 여기서 알 수 없다.
///
/// ★무리가 붙은 채 마지막 것을 놓지 않는다 — 먼저 떼고([`ProcessGroup::unwatch_births`]) 놓는다★: 붙어 있는 동안
/// 무리가 포트를 제 커널 참조로 쥐어 핸들이 닫혀도 포트가 남고, 아무도 안 읽는 포트에 알림이 계속 쌓인다. 그 커널
/// 참조는 MS Learn 에 적혀 있지 않고 실측하지 않았다 — 틀려도 먼저 떼는 쪽은 잃는 것이 없다.
pub(crate) trait Births: Send + Sync {
    /// 알림 하나를 `wait` 까지 기다려 꺼낸다. `Err` = 꺼내기 자체의 실패(포트가 닫힘 등).
    fn next(&self, wait: Duration) -> io::Result<PortEvent>;
}

/// `Gone` = 붙든 뒤 끝내기 전에 스스로 끝났다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MemberKill {
    Terminated,
    Gone,
}

/// 통로가 이미 사라져 무리가 없을 때 [`ProcessGroup::watch_births`] · [`ProcessGroup::unwatch_births`] 가 돌려주는
/// `Err` 종류 — OS 실패가 아니라서 포트 실패로 굳히지 않는다(TRD t40 §3-5). 포트 만들기 · 붙이기 · 떼기의 OS 실패는
/// 이 종류가 아니다(`Other` 로 감싼다) — 단 `start` 가 이 종류를 돌려주면 가를 수 없다.
pub(crate) const GROUP_GONE: io::ErrorKind = io::ErrorKind::NotConnected;

pub(crate) struct ProcessGroup {
    #[cfg(windows)]
    job: Weak<JobObjectHandle>,
    retiring: RetiringSignal,
}

impl ProcessGroup {
    #[cfg(windows)]
    pub(crate) fn new(job: Weak<JobObjectHandle>, retiring: RetiringSignal) -> Self {
        Self { job, retiring }
    }

    /// 무리가 이미 사라진 손잡이 — 모든 OS 에서 같은 모양이다.
    #[cfg(test)]
    pub(crate) fn detached(retiring: RetiringSignal) -> Self {
        Self {
            #[cfg(windows)]
            job: Weak::new(),
            retiring,
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

    /// 그 번호의 프로세스를 **우리 무리의 멤버일 때만** 붙들고 사실을 한 번 읽는다. 살았는지는 보지 않는다.
    ///
    /// `kill` = 끝내기 권한도 연다. `Ok(None)` = 그 번호의 프로세스가 없다 · 우리 무리 밖이다 · 통로가 이미
    /// 사라졌다 · 이 OS 에는 무리가 없다. `Err` = 여는 실패(부재 말고 — 권한 거부 등) · 소속 조회 실패.
    pub(crate) fn pin(&self, pid: u32, kill: bool) -> io::Result<Option<Box<dyn Pinned>>> {
        #[cfg(windows)]
        {
            let Some(job) = self.job.upgrade() else {
                return Ok(None);
            };
            Ok(job.pin_member(pid, kill)?.map(|member| {
                let facts = ProcessFacts::from(member.facts());
                Box::new(JobPin { member, facts }) as Box<dyn Pinned>
            }))
        }
        #[cfg(not(windows))]
        {
            let _ = (pid, kill);
            Ok(None)
        }
    }

    /// 가입 알림 포트를 만들어 `start` 에 넘기고, **`start` 가 `Ok` 일 때만** 무리를 그 포트에 붙인다 — 알림이 쌓일
    /// 수 있게 되는 때에는 꺼낼 쪽이 이미 포트를 쥐고 있다. `start` 의 `Err` 는 그대로 돌려준다(붙지 않았다).
    ///
    /// ★`start` 가 도는 동안 Job 을 강하게 쥔다(올린 `Arc`)★ — 그 사이 통로가 사라져도 Job 핸들이 안 닫혀
    /// `KILL_ON_JOB_CLOSE` 가 `start` 가 막힌 만큼 늦는다. 그래서 `start` 는 꺼낼 스레드를 띄우기만 하고 막히지 않는다.
    ///
    /// 붙일 때 이미 든 멤버도 `Joined` 로 다시 알려진다. ★한 무리에 한 번만 부른다★ — 두 번째 부름을 여기서 막지
    /// 않는다(부르는 쪽 몫). `Err` 종류: [`GROUP_GONE`] = 통로가 이미 사라졌다(`start` 는 불리지 않았다) ·
    /// `Unsupported` = 이 OS 에는 무리가 없다(`start` 는 불리지 않았다) · 그 밖 = `start` 의 오류 · 포트 만들기 ·
    /// 붙이기의 OS 실패.
    pub(crate) fn watch_births(
        &self,
        start: impl FnOnce(Arc<dyn Births>) -> io::Result<()>,
    ) -> io::Result<()> {
        #[cfg(windows)]
        {
            let Some(job) = self.job.upgrade() else {
                return Err(group_gone());
            };
            job.watch_births(|port| {
                let port: Arc<dyn Births> = port;
                start(port)
            })
        }
        #[cfg(not(windows))]
        {
            drop(start);
            Err(no_group_here())
        }
    }

    /// 무리의 포트 연결을 뗀다 — 뗀 뒤 태어난 멤버는 알려지지 않는다. `Err` 종류는 [`Self::watch_births`] 와 같다.
    pub(crate) fn unwatch_births(&self) -> io::Result<()> {
        #[cfg(windows)]
        {
            let Some(job) = self.job.upgrade() else {
                return Err(group_gone());
            };
            job.unwatch_births()
        }
        #[cfg(not(windows))]
        {
            Err(no_group_here())
        }
    }

    pub(crate) fn retiring(&self) -> RetiringSignal {
        self.retiring.clone()
    }
}

#[cfg(windows)]
impl From<&MemberFacts> for ProcessFacts {
    fn from(facts: &MemberFacts) -> Self {
        Self {
            ppid: facts.ppid,
            create: facts.create,
            image: facts.image.clone(),
            cmdline: facts.cmdline.clone(),
        }
    }
}

#[cfg(windows)]
struct JobPin {
    member: PinnedMember,
    facts: ProcessFacts,
}

#[cfg(windows)]
impl Pinned for JobPin {
    fn exited(&self) -> io::Result<bool> {
        self.member.exited()
    }

    fn wait_exit(&self, d: Duration) -> io::Result<bool> {
        self.member.wait_exit(d)
    }

    fn facts(&self) -> &ProcessFacts {
        &self.facts
    }

    fn terminate_raw(&self) -> io::Result<()> {
        self.member.terminate_raw()
    }

    fn classify(&self, raw: io::Result<()>) -> io::Result<MemberKill> {
        Ok(match self.member.classify(raw)? {
            MemberOutcome::Terminated => MemberKill::Terminated,
            MemberOutcome::Gone => MemberKill::Gone,
        })
    }
}

#[cfg(windows)]
impl Births for BirthPort {
    fn next(&self, wait: Duration) -> io::Result<PortEvent> {
        // 경로 호출은 고유 메서드를 먼저 고른다 — 이 trait 메서드로 되돌아오지 않는다.
        Ok(match BirthPort::next(self, wait)? {
            PortOutcome::Joined(pid) => PortEvent::Joined(pid),
            PortOutcome::Other => PortEvent::Other,
            PortOutcome::Timeout => PortEvent::Timeout,
        })
    }
}

#[cfg(windows)]
fn group_gone() -> io::Error {
    io::Error::new(GROUP_GONE, "통로가 이미 사라졌다 — 무리가 없다")
}

#[cfg(not(windows))]
fn no_group_here() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "이 OS 에는 프로세스 무리가 없다",
    )
}

// 공용 실프로세스 도우미는 `windows.rs` 의 시험에 있다(그 파일은 이 모듈을 부르지 않는다). claude 잔여물 정리의 실물
// 시험은 여기 두지 않는다 — 두면 이 층이 claude 를 부른다.
#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// 통로가 사라진 손잡이 · 비Windows 껍데기가 같은 답을 준다 — 멤버 없음 · 아무도 못 붙듦 · 포트 없음(`start` 를
    /// 부르지 않는다). 종류만 OS 마다 갈린다.
    #[test]
    fn a_group_without_a_live_job_lists_nothing_and_owns_nobody() {
        let group = ProcessGroup::detached(RetiringSignal::of(&Arc::default()));
        assert_eq!(group.member_pids().expect("명단"), Vec::<u32>::new());
        for kill in [false, true] {
            assert!(group
                .pin(std::process::id(), kill)
                .expect("붙들기")
                .is_none());
        }

        let refused = if cfg!(windows) {
            GROUP_GONE
        } else {
            io::ErrorKind::Unsupported
        };
        let mut started = false;
        let watched = group.watch_births(|_| {
            started = true;
            Ok(())
        });
        assert_eq!(watched.expect_err("무리가 없는데 붙었다").kind(), refused);
        assert!(!started, "무리가 없는데 꺼낼 쪽을 띄웠다");
        assert_eq!(
            group
                .unwatch_births()
                .expect_err("무리가 없는데 뗐다")
                .kind(),
            refused
        );
    }

    /// 내준 표시는 칸의 주인이 세운 것을 따른다 — 무리 손잡이를 거쳐도, 복제해도 같은 칸이다.
    #[test]
    fn a_retiring_signal_follows_the_flag_it_shares() {
        let flag = Arc::new(AtomicBool::new(false));
        let group = ProcessGroup::detached(RetiringSignal::of(&flag));
        let signal = group.retiring();
        let copy = signal.clone();
        assert!(!signal.is_set());
        assert!(!group.retiring().is_set());

        flag.store(true, Ordering::Release);
        assert!(signal.is_set());
        assert!(copy.is_set());
        assert!(group.retiring().is_set());
    }

    // ── 실프로세스(Windows) — 시험마다 몇 개만 차례로 띄운다: 몰아 띄우면 개발 PC 터미널이 죽는다(CLAUDE.md) ──

    #[cfg(windows)]
    use std::io::Write;
    #[cfg(windows)]
    use std::process::{Child, ChildStdin, Command, Stdio};
    #[cfg(windows)]
    use std::time::Instant;

    #[cfg(windows)]
    use crate::platform::windows::tests::{is_ping, open_gate, spawn_gated_cmd, wait_until};
    #[cfg(windows)]
    use crate::platform::windows::LEFTOVER_EXIT_CODE;

    #[cfg(windows)]
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    #[cfg(windows)]
    const DETACHED_PROCESS: u32 = 0x0000_0008;

    #[cfg(windows)]
    pub(crate) fn new_group() -> (Arc<JobObjectHandle>, ProcessGroup) {
        let job = Arc::new(JobObjectHandle::new().expect("Job 생성"));
        let group = ProcessGroup::new(Arc::downgrade(&job), RetiringSignal::of(&Arc::default()));
        (job, group)
    }

    /// [`open_gate`] 와 같되 stdin 을 닫지 않고 돌려준다 — `line` 이 stdin 을 다시 읽으면(`set /p`) 돌려준 끝을
    /// 쥐는 동안 거기서 기다린다.
    #[cfg(windows)]
    fn open_gate_keeping_stdin(child: &mut Child) -> ChildStdin {
        let mut stdin = child.stdin.take().expect("stdin 파이프");
        stdin.write_all(b"go\r\n").expect("문 열기");
        stdin
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
    fn pin_ours(group: &ProcessGroup, pid: u32, kill: bool) -> Box<dyn Pinned> {
        group
            .pin(pid, kill)
            .expect("붙들기")
            .unwrap_or_else(|| panic!("PID {pid} 가 우리 무리의 멤버가 아니다"))
    }

    /// 붙든 멤버의 부모가 끝났나 — 적힌 부모 번호를 우리 무리에서 붙들 수 없다 · 붙든 부모가 끝났다 · 그 번호를
    /// 자기보다 늦게 태어난 것이 쥐었다. ★부모가 우리 무리에서 태어났다는 전제다★ — 무리 밖의 산 부모도 「끝남」
    /// 으로 읽힌다. 못 붙들면(권한 거부 등) 끝남으로 치지 않는다.
    #[cfg(windows)]
    fn parent_is_dead(group: &ProcessGroup, member: &dyn Pinned) -> bool {
        let child = member.facts();
        assert!(
            child.ppid != 0 && child.create != 0,
            "부모 번호나 생성 시각을 못 읽었다: {child:?}"
        );
        match group.pin(child.ppid, false) {
            Ok(None) => true,
            Ok(Some(parent)) => {
                parent.exited().expect("끝났나") || parent.facts().create > child.create
            }
            Err(_) => false,
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

    /// ② Job 안 cmd 의 자식 ping 을 끝내기 권한으로 붙들어 끝낸다 — 붙든 핸들로 끝남을 기다리고, 종료 코드로 누가
    /// 끝냈는지 남는다. 끝난 뒤에도 쥐고 있어 그 번호는 명단 확인 동안 재사용되지 않는다.
    #[cfg(windows)]
    #[test]
    fn a_pinned_member_is_terminated_with_the_leftover_code() {
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
        let watch = Watch::open(ping);
        let member = pin_ours(&group, ping, true);
        assert!(!member.exited().expect("끝났나"));

        let raw = member.terminate_raw();
        assert_eq!(
            member.classify(raw).expect("끝내기"),
            MemberKill::Terminated
        );
        assert!(
            member
                .wait_exit(Duration::from_secs(5))
                .expect("끝나기 대기"),
            "5 초 안에 안 끝났다"
        );
        assert!(member.exited().expect("끝났나"));
        assert_eq!(
            watch.exit_code_within(Duration::ZERO),
            Some(LEFTOVER_EXIT_CODE)
        );
        drop(watch);
        wait_until("명단에서 빠진 ping", || {
            (!group.member_pids().ok()?.contains(&ping)).then_some(())
        });
        drop(member);
        let _ = root.wait();
    }

    /// ③ 다른 Job 의 산 프로세스는 붙들지 않는다(조사 §4 사고의 회귀망).
    #[cfg(windows)]
    #[test]
    fn a_process_in_another_job_is_not_ours() {
        let (_ours, group) = new_group();
        let other = JobObjectHandle::new().expect("남의 Job");
        let mut stranger = spawn_lone_ping();
        other.assign(stranger.id()).expect("남의 Job 편입");

        for kill in [false, true] {
            assert!(
                group.pin(stranger.id(), kill).expect("붙들기").is_none(),
                "남의 Job 멤버를 붙들었다(끝내기 권한 {kill})"
            );
        }
        assert!(stranger.try_wait().expect("상태").is_none(), "남이 죽었다");

        let _ = stranger.kill();
        let _ = stranger.wait();
    }

    /// ③ 중첩 변형 — 우리 Job 이 부모 Job P 아래로 중첩돼도 P 에만 든 남은 우리 것이 아니다. Job 인자로 NULL 을
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
            group.pin(member.id(), true).expect("붙들기").is_some(),
            "중첩이 안 섰다 — 우리 멤버를 못 붙들었다"
        );
        assert!(
            group.pin(stranger.id(), true).expect("붙들기").is_none(),
            "부모 Job 에만 든 남을 붙들었다"
        );
        assert!(stranger.try_wait().expect("상태").is_none(), "남이 죽었다");

        for child in [&mut member, &mut stranger] {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// ⑦ 손잡이를 거친 포트 — `start` 의 오류는 그대로 오고(붙지 않음), 받은 포트에서 붙일 때 이미 든 멤버가
    /// `Joined` 로 다시 알려진다. 붙이기 · 떼기의 OS 동작 자체는 `windows.rs` 의 ⑦ 이 잰다.
    #[cfg(windows)]
    #[test]
    fn a_port_through_the_group_reports_the_members_already_in() {
        let (job, group) = new_group();
        let mut ping = spawn_lone_ping();
        job.assign(ping.id()).expect("Job 편입");

        let refused = group.watch_births(|_| Err(io::Error::other("기동 실패 흉내")));
        assert_eq!(
            refused
                .expect_err("start 의 실패가 그대로 와야 한다")
                .to_string(),
            "기동 실패 흉내"
        );

        let mut held = None;
        group
            .watch_births(|port| {
                held = Some(port);
                Ok(())
            })
            .expect("붙이기");
        let port = held.expect("start 가 포트를 받았다");
        let deadline = Instant::now() + Duration::from_secs(10);
        while port.next(Duration::from_millis(100)).expect("꺼내기") != PortEvent::Joined(ping.id())
        {
            assert!(
                Instant::now() < deadline,
                "10 초 안에 ping 의 되알림을 못 봤다"
            );
        }
        group.unwatch_births().expect("떼기");

        let _ = ping.kill();
        let _ = ping.wait();
    }

    /// ① 끝났지만 핸들이 붙든(좀비) 부모 — Job 명단에서 빠지고 명단은 완전하다(좀비가 멤버 수에만 남으면 명단이
    /// 영영 불완전해 정리가 한 번도 돌지 못한다). 붙든 자식에서 보면 그 부모는 끝났다 — 좀비가 산 것으로 읽히면
    /// 잔여물을 놓친다.
    #[cfg(windows)]
    #[test]
    fn a_dead_parent_held_open_is_off_the_list_and_reads_dead() {
        let (job, group) = new_group();
        // P 는 ping 을 띄운 뒤 둘째 `set /p` 에서 stdin 을 기다리며 산다 — 끝은 아래 kill 한 곳이다. 둘째 `set /p`
        // 가 없으면 `cmd /c` 가 `start /b` 직후 끝나 P 가 탐침 전에 이미 죽어 있다(실측 5/5).
        let mut parent = spawn_gated_cmd("start \"\" /b ping -n 30 127.0.0.1 & set /p _=", 0);
        let parent_pid = parent.id();
        job.assign(parent_pid).expect("Job 편입");
        let gate = open_gate_keeping_stdin(&mut parent);
        let (child_pid, child) = wait_until("P 가 띄운 ping", || {
            group
                .member_pids()
                .ok()?
                .into_iter()
                .filter(|&pid| is_ping(pid))
                .find_map(|pid| {
                    let member = group.pin(pid, false).ok()??;
                    (member.facts().ppid == parent_pid).then_some((pid, member))
                })
        });
        assert!(
            parent.try_wait().expect("상태").is_none(),
            "P 가 탐침 전에 끝났다"
        );
        assert!(
            !parent_is_dead(&group, child.as_ref()),
            "산 P 가 끝남으로 읽힌다 — 아래 단언이 뜻이 없다"
        );
        let _ = parent.kill();
        let _ = parent.wait();
        drop(gate);
        // `parent` 를 아직 쥐고 있다 — 그 핸들이 P 의 프로세스 객체를 붙든다.

        let members = group
            .member_pids()
            .expect("끝났지만 핸들이 붙든 멤버가 있어도 명단은 완전해야 한다");
        assert!(
            !members.contains(&parent_pid),
            "끝났지만 핸들이 붙든 P 가 명단에 남았다: {members:?}"
        );
        assert!(
            members.contains(&child_pid),
            "산 C 가 명단에 없다: {members:?}"
        );
        assert!(
            parent_is_dead(&group, child.as_ref()),
            "C 의 부모가 끝남으로 안 읽힌다"
        );
        drop(parent);
    }

    /// ① 첫 칸 하나로 물어도 늘려 되물어 전부 나온다.
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

    /// ⑤ 부모가 끝난 MSYS fork 자식이 우리 Job 멤버로 남는지 · 부모 끝남으로 읽히는지 · 끝나는지.
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
            let found: Vec<(u32, Box<dyn Pinned>)> = group
                .member_pids()
                .ok()?
                .into_iter()
                .filter_map(|pid| Some((pid, group.pin(pid, true).ok()??)))
                .filter(|(_, m)| {
                    m.facts()
                        .image
                        .to_ascii_lowercase()
                        .contains("\\usr\\bin\\")
                })
                .filter(|(_, m)| parent_is_dead(&group, m.as_ref()))
                .collect();
            (!found.is_empty()).then_some(found)
        });
        for (pid, orphan) in orphans {
            let watch = Watch::open(pid);
            let raw = orphan.terminate_raw();
            assert_eq!(
                orphan.classify(raw).expect("끝내기"),
                MemberKill::Terminated
            );
            assert_eq!(
                watch.exit_code_within(Duration::from_secs(5)),
                Some(LEFTOVER_EXIT_CODE)
            );
            assert!(orphan.exited().expect("끝났나"));
        }
    }
}
