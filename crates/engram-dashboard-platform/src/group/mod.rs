//! 한 무리로 묶은 프로세스의 OS 층 — 강한 주인 [`GroupOwner`] · 약한 손잡이 [`GroupRef`] 와 손잡이가 내주는
//! 구체 타입(붙든 멤버 [`PinnedMember`] · 가입 알림 포트 [`BirthPort`] · 값 [`MemberFacts`] · [`PortOutcome`] ·
//! [`MemberOutcome`]). Windows 는 Job Object(비공개 `windows` 조각 — 이 모듈이 그 타입을 제 공개 타입으로 옮겨
//! 싣는다)이고, 그 밖의 OS 는 아무것도 묶지 않는 껍데기다(각 메서드 문서).
//!
//! 무엇을 끝낼지는 모른다(ADR-0266 결정 3) — 고르기 · 물러남 같은 뜻은 부르는 쪽 어댑터의 몫이고, 여기는 받은
//! 번호가 그 무리의 멤버인지 OS 에 물어 붙들고 그 핸들로 사실을 읽고 끝낼 뿐이다. 끝낼 때 줄 종료 코드도 부르는
//! 쪽이 준다(ADR-0275 결정 12). 공개 API 에 트레이트 객체가 없다 — 시험용 가짜가 필요하면 부르는 쪽이 작은
//! 트레이트를 두고 이 구체 타입에 구현한다(ADR-0266 결정 5).
//!
//! ★로그도 락도 쓰지 않는다★ — [`PinnedMember::terminate_raw`] · [`PinnedMember::classify`] · [`GroupRef::is_gone`]
//! 은 부르는 쪽이 자물쇠를 쥔 채 부르고, 그 자물쇠 안의 OS 호출은 끝내기 하나로 묶여 있다(ADR-0262 결정 8). 이
//! crate 가 `tracing` 을 쓸 수 있다는 것(ADR-0266 결정 9)을 이 모듈로 번지게 하지 말 것.
// ADR-0262
// ADR-0266
// ADR-0275

use std::io;
use std::sync::Arc;
#[cfg(windows)]
use std::sync::Weak;
use std::time::Duration;

#[cfg(windows)]
mod windows;

#[cfg(all(windows, any(test, feature = "test-support")))]
pub(crate) use self::windows::image_path;
#[cfg(windows)]
pub use self::windows::resume_suspended_process;

/// 주인이 이미 사라져 무리가 없을 때 [`GroupRef::watch_births`] · [`GroupRef::unwatch_births`] 가 돌려주는 `Err`
/// 종류 — OS 실패가 아니라서 부르는 쪽이 가를 수 있게 따로 둔다. 포트 만들기 · 붙이기 · 떼기의 OS 실패는 이
/// 종류가 아니다(`Other` 로 감싼다) — 단 `start` 가 이 종류를 돌려주면 가를 수 없다.
pub const GROUP_GONE: io::ErrorKind = io::ErrorKind::NotConnected;

/// 무리의 강한 주인 — 이것이 살아 있는 동안 무리가 있다. 버리면 Windows 에서는 Job 핸들이 닫혀 남은 멤버를 OS 가
/// 끝낸다(`KILL_ON_JOB_CLOSE`) — 단 그 순간 [`GroupRef`] 가 부르는 중이면 그 호출이 끝날 때 닫힌다.
///
/// ★`Clone` 이 아니다★ — 타입이 강제하는 것은 「한 무리의 강한 주인은 하나」까지다. 그 주인이 누구인가(띄운
/// 통로 · 트리 손잡이)는 배치가 지킨다(ADR-0275 결정 11). 밖으로는 [`Self::downgrade`] 의 약한 손잡이만 내준다.
///
/// Windows 밖에서는 아무것도 묶지 않는다 — `new` · `adopt` · `terminate` 는 `Ok`, `active_processes` 는
/// `Unsupported`, `downgrade` 는 `None` 이다.
// ADR-0266
// ADR-0275
pub struct GroupOwner {
    #[cfg(windows)]
    job: Arc<windows::JobObjectHandle>,
}

impl GroupOwner {
    /// Windows = 익명 Job 을 `KILL_ON_JOB_CLOSE` 로 만든다. `Err` = 만들기 · 설정의 OS 실패.
    pub fn new() -> io::Result<Self> {
        #[cfg(windows)]
        {
            Ok(Self {
                job: Arc::new(windows::JobObjectHandle::new()?),
            })
        }
        #[cfg(not(windows))]
        {
            Ok(Self {})
        }
    }

    /// 그 번호의 프로세스를 무리에 넣는다.
    ///
    /// ★띄운 뒤에 넣으면 넣기 전에 그것이 띄운 자식은 무리 밖에 남는다★ — 닫는 법 = 멈춘 채 띄워 넣은 뒤
    /// `resume_suspended_process` 로 깨운다.
    pub fn adopt(&self, pid: u32) -> io::Result<()> {
        #[cfg(windows)]
        {
            self.job.assign(pid)
        }
        #[cfg(not(windows))]
        {
            let _ = pid;
            Ok(())
        }
    }

    /// 멤버 전부를 `exit_code` 로 끝낸다 — 끝내기를 시작만 하고 돌아온다. 다 끝났는지는
    /// [`Self::active_processes`] 로 본다.
    // ADR-0001
    pub fn terminate(&self, exit_code: u32) -> io::Result<()> {
        #[cfg(windows)]
        {
            self.job.terminate(exit_code)
        }
        #[cfg(not(windows))]
        {
            let _ = exit_code;
            Ok(())
        }
    }

    /// 무리 안에서 아직 끝나지 않은 프로세스 수. Windows 밖 = `Unsupported`.
    pub fn active_processes(&self) -> io::Result<u32> {
        #[cfg(windows)]
        {
            self.job.active_processes()
        }
        #[cfg(not(windows))]
        {
            Err(no_group_here())
        }
    }

    /// 무리를 붙들지 않는 약한 손잡이. `None` = 이 OS 에는 무리를 묶는 수단이 없다(Windows 밖).
    pub fn downgrade(&self) -> Option<GroupRef> {
        #[cfg(windows)]
        {
            Some(GroupRef {
                job: Arc::downgrade(&self.job),
            })
        }
        #[cfg(not(windows))]
        {
            None
        }
    }
}

/// 무리의 약한 손잡이 — 멤버 명단 · 멤버 하나 붙들기(사실 · 끝내기) · 가입 알림 포트 · 「사라졌나」.
///
/// ★무리를 약하게 쥔다★ — 강하게 쥐면 주인이 사라진 뒤에도 Job 핸들이 안 닫혀 `KILL_ON_JOB_CLOSE` 가 늦어진다.
/// 부를 때마다 그 호출 동안만 올린다([`Self::is_gone`] 은 올리지도 않는다) — 가장 긴 창은 [`Self::watch_births`]
/// 가 `start` 를 부르는 동안이다(그래서 `start` 는 막히지 않아야 한다). 주인이 사라졌으면 명단은 비고 아무도 못
/// 붙들며 포트는 [`GROUP_GONE`] 이다. 내준 붙든 멤버 · 포트는 무리를 붙들지 않는다.
///
/// Windows 밖에서는 주인이 사라진 손잡이와 같은 답이다 — 포트의 `Err` 종류만 `Unsupported` 다.
// ADR-0266
// ADR-0275
pub struct GroupRef {
    #[cfg(windows)]
    job: Weak<windows::JobObjectHandle>,
}

impl GroupRef {
    /// 주인이 처음부터 없는 손잡이 — 모든 OS 에서 같은 모양이다.
    #[cfg(any(test, feature = "test-support"))]
    pub fn gone() -> Self {
        Self {
            #[cfg(windows)]
            job: Weak::new(),
        }
    }

    /// 완전한 멤버 PID 명단(뿌리 포함 · 중첩된 무리의 멤버 포함 · 순서는 뜻이 없다). 주인이 사라졌으면
    /// `Ok(빈 목록)` · 되묻는 상한 안에 명단이 완전해지지 않으면 `Err` — 빠진 멤버를 「무리 밖」으로 읽지 않게.
    ///
    /// ★막 끝난 멤버가 잠깐 남을 수 있다★ — 명단에서 빠지는 때가 끝남(붙든 핸들의 대기가 돌아옴)보다 늦을 수
    /// 있다(CI 실측 2026-10-01). 살았는지는 명단이 아니라 붙든 핸들([`PinnedMember::exited`])로 본다.
    pub fn member_pids(&self) -> io::Result<Vec<u32>> {
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

    /// 그 번호의 프로세스를 **이 무리의 멤버일 때만** 붙들고 사실을 한 번 읽는다. 살았는지는 보지 않는다.
    ///
    /// `kill` = 끝내기 권한도 연다. `Ok(None)` = 그 번호의 프로세스가 없다 · 이 무리 밖이다(이 무리를 감싼 바깥
    /// Job 에만 든 것 포함) · 주인이 이미 사라졌다 · 이 OS 에는 무리가 없다. `Err` = 여는 실패(부재 말고 — 권한
    /// 거부 등) · 소속 조회 실패.
    pub fn pin_member(&self, pid: u32, kill: bool) -> io::Result<Option<PinnedMember>> {
        #[cfg(windows)]
        {
            let Some(job) = self.job.upgrade() else {
                return Ok(None);
            };
            Ok(job.pin_member(pid, kill)?.map(|os| PinnedMember {
                facts: facts_from(os.facts()),
                os,
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
    /// ★`start` 가 도는 동안 무리를 강하게 쥔다(올린 `Arc`)★ — 그 사이 주인이 사라져도 Job 핸들이 안 닫혀
    /// `KILL_ON_JOB_CLOSE` 가 `start` 가 막힌 만큼 늦는다. 그래서 `start` 는 꺼낼 스레드를 띄우기만 하고 막히지 않는다.
    ///
    /// 붙일 때 이미 든 멤버도 `Joined` 로 다시 알려진다 · 붙이는 동안 상태가 바뀐 멤버의 알림은 빠질 수 있다(MS
    /// Learn). ★한 무리에 한 번만 부른다★ — 두 번째 부름을 여기서 막지 않는다(부르는 쪽 몫). `Err` 종류:
    /// [`GROUP_GONE`] = 주인이 이미 사라졌다(`start` 는 불리지 않았다) · `Unsupported` = 이 OS 에는 무리가 없다
    /// (`start` 는 불리지 않았다) · 그 밖 = `start` 의 오류 · 포트 만들기 · 붙이기의 OS 실패.
    pub fn watch_births(
        &self,
        start: impl FnOnce(Arc<BirthPort>) -> io::Result<()>,
    ) -> io::Result<()> {
        #[cfg(windows)]
        {
            let Some(job) = self.job.upgrade() else {
                return Err(group_gone());
            };
            job.watch_births(|os| start(Arc::new(BirthPort { os })))
        }
        #[cfg(not(windows))]
        {
            drop(start);
            Err(no_group_here())
        }
    }

    /// 무리의 포트 연결을 뗀다 — 뗀 뒤 태어난 멤버는 알려지지 않는다. `Err` 종류는 [`Self::watch_births`] 와 같다.
    pub fn unwatch_births(&self) -> io::Result<()> {
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

    /// 주인이 사라져 무리가 없나 — 무리를 올리지 않고 본다(부작용 없음 · 그 사이 Job 핸들 닫기를 늦추지 않는다). 한
    /// 번 참이면 계속 참이다. 이 OS 에는 무리가 없어 늘 참이다.
    pub fn is_gone(&self) -> bool {
        #[cfg(windows)]
        {
            self.job.strong_count() == 0
        }
        #[cfg(not(windows))]
        {
            true
        }
    }
}

/// 붙든 핸들로 **한 번** 읽은 사실 — 번호가 아니라 그 프로세스 객체의 것이다. 못 읽은 칸은 `ppid` · `create` =
/// 0 · 문자열 = 빈 문자열이다.
///
/// - `ppid` = 만들 때 적힌 부모 번호 — 그 번호가 지금도 같은 부모라는 보장은 없다.
/// - `create` = 생성 시각(FILETIME · 100 ns) — 두 프로세스의 생성 순서를 비교하는 데만 뜻이 있다.
/// - `image` = 실행 파일 전체 경로(Win32 형식) · `cmdline` = 명령줄 그대로.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemberFacts {
    pub ppid: u32,
    pub create: u64,
    pub image: String,
    pub cmdline: String,
}

/// [`BirthPort::next`] 의 답.
///
/// - `Joined(pid)` = 멤버가 새로 들었다. 번호뿐이라 꺼낸 때 이미 끝나 재사용됐을 수 있다 — 곧바로 붙들어야
///   신원이 선다.
/// - `Other` = 그 밖의 알림(끝남 등) 또는 이 포트의 것이 아닌 패킷.
/// - `Timeout` = 기다리는 동안 알림이 없었다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortOutcome {
    Joined(u32),
    Other,
    Timeout,
}

/// [`PinnedMember::classify`] 의 답. `Gone` = 붙든 뒤 끝내기 전에 스스로 끝났다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberOutcome {
    Terminated,
    Gone,
}

/// 이 무리의 멤버로 확인하고 붙든 프로세스 하나 — 쥐는 동안 그 번호는 재사용되지 않고, 끝난 뒤에도 사실은
/// 남는다. 무리를 붙들지 않는다. 버리면 핸들이 닫힌다. [`GroupRef::pin_member`] 만 만든다.
///
/// 끝내기가 두 메서드로 갈린 것은 부르는 쪽이 자물쇠 안에서 OS 호출을 **하나만** 하게 하려는 것이다:
/// [`Self::terminate_raw`] = 끝내기 호출 하나 · [`Self::classify`] = 그 날것 결과 가르기.
#[derive(Debug)]
pub struct PinnedMember {
    #[cfg(windows)]
    os: windows::PinnedMember,
    // Windows 밖에서는 만들어지지 않는다 — 메서드 몸이 빈 match 로 그것을 말한다.
    #[cfg(not(windows))]
    os: std::convert::Infallible,
    facts: MemberFacts,
}

impl PinnedMember {
    /// 지금 끝났나 — 기다리지 않는다. `Err` = 대기 조회 자체의 실패.
    pub fn exited(&self) -> io::Result<bool> {
        #[cfg(windows)]
        {
            self.os.exited()
        }
        #[cfg(not(windows))]
        {
            match self.os {}
        }
    }

    /// `limit` 까지 끝나기를 기다린다 — 끝났으면 `true`. `Err` = 대기 조회 자체의 실패.
    pub fn wait_exit(&self, limit: Duration) -> io::Result<bool> {
        #[cfg(windows)]
        {
            self.os.wait_exit(limit)
        }
        #[cfg(not(windows))]
        {
            let _ = limit;
            match self.os {}
        }
    }

    pub fn facts(&self) -> &MemberFacts {
        &self.facts
    }

    /// 붙든 그 핸들로 `exit_code` 를 주는 끝내기 호출 **하나만** 한다 — 대상이 끝나기를 기다리지 않는다. 날것
    /// 결과만 돌려주며 가르기는 [`Self::classify`] 가 한다. 끝내기 권한 없이 붙든 것(`kill = false`)은 늘 `Err`
    /// 다. 실패도 힙을 쓰지 않는 오류로 돌려준다.
    // ADR-0262
    pub fn terminate_raw(&self, exit_code: u32) -> io::Result<()> {
        #[cfg(windows)]
        {
            self.os.terminate_raw(exit_code)
        }
        #[cfg(not(windows))]
        {
            let _ = exit_code;
            match self.os {}
        }
    }

    /// [`Self::terminate_raw`] 의 날것 결과를 가른다 — 실패여도 그 사이 스스로 끝났으면 `Gone` 이다. 핸들은
    /// 닫지 않는다(버릴 때 닫힌다).
    // ADR-0262
    pub fn classify(&self, raw: io::Result<()>) -> io::Result<MemberOutcome> {
        #[cfg(windows)]
        {
            Ok(match self.os.classify(raw)? {
                windows::MemberOutcome::Terminated => MemberOutcome::Terminated,
                windows::MemberOutcome::Gone => MemberOutcome::Gone,
            })
        }
        #[cfg(not(windows))]
        {
            let _ = raw;
            match self.os {}
        }
    }
}

/// 무리의 가입 알림 포트 — [`GroupRef::watch_births`] 가 `start` 에 넘긴다. 여러 스레드가 `Arc` 로 나눠 쥐고,
/// 마지막 것이 사라지면 포트 핸들이 닫힌다. 무리를 붙들지 않는다.
///
/// 알림은 통지일 뿐 배달이 보장되지 않는다(MS Learn) — 빠진 가입은 여기서 알 수 없다.
///
/// ★무리가 붙은 채 마지막 것을 놓지 않는다 — 먼저 떼고([`GroupRef::unwatch_births`]) 놓는다★: 붙어 있는 동안
/// 무리가 포트를 제 커널 참조로 쥐어 핸들이 닫혀도 포트가 남고, 아무도 안 읽는 포트에 알림이 계속 쌓인다. 그 커널
/// 참조는 MS Learn 에 적혀 있지 않고 실측하지 않았다 — 틀려도 먼저 떼는 쪽은 잃는 것이 없다.
#[derive(Debug)]
pub struct BirthPort {
    // 안쪽 `Arc` 는 조각의 `watch_births` 가 붙이기를 마칠 때까지 쥐는 제 몫과 나눈다 — 그 전에 핸들이 닫히면 그
    //   값이 재사용돼 남의 핸들을 붙일 수 있다(그 함수 주석).
    #[cfg(windows)]
    os: Arc<windows::BirthPort>,
    #[cfg(not(windows))]
    os: std::convert::Infallible,
}

impl BirthPort {
    /// 알림 하나를 `wait` 까지 기다려 꺼낸다. `Err` = 꺼내기 자체의 실패(포트가 닫힘 등).
    pub fn next(&self, wait: Duration) -> io::Result<PortOutcome> {
        #[cfg(windows)]
        {
            Ok(match self.os.next(wait)? {
                windows::PortOutcome::Joined(pid) => PortOutcome::Joined(pid),
                windows::PortOutcome::Other => PortOutcome::Other,
                windows::PortOutcome::Timeout => PortOutcome::Timeout,
            })
        }
        #[cfg(not(windows))]
        {
            let _ = wait;
            match self.os {}
        }
    }
}

#[cfg(windows)]
fn facts_from(os: &windows::MemberFacts) -> MemberFacts {
    MemberFacts {
        ppid: os.ppid,
        create: os.create,
        image: os.image.clone(),
        cmdline: os.cmdline.clone(),
    }
}

#[cfg(windows)]
fn group_gone() -> io::Error {
    io::Error::new(GROUP_GONE, "무리의 주인이 이미 사라졌다")
}

#[cfg(not(windows))]
fn no_group_here() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "이 OS 에는 프로세스 무리가 없다",
    )
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::{GroupRef, GROUP_GONE};

    /// 주인이 없는 손잡이가 주는 답 — 멤버 없음 · 아무도 못 붙듦 · 포트 없음(`start` 를 부르지 않는다). 종류만 OS
    /// 마다 갈린다.
    #[test]
    fn a_group_without_a_live_job_lists_nothing_and_owns_nobody() {
        let group = GroupRef::gone();
        assert!(group.is_gone());
        assert_eq!(group.member_pids().expect("명단"), Vec::<u32>::new());
        for kill in [false, true] {
            assert!(group
                .pin_member(std::process::id(), kill)
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

    // ── 실프로세스(Windows) — 시험마다 몇 개만 차례로 띄운다: 몰아 띄우면 개발 PC 터미널이 죽는다(CLAUDE.md) ──

    #[cfg(windows)]
    use std::io::Write;
    #[cfg(windows)]
    use std::process::{Child, ChildStdin, Command, Stdio};
    #[cfg(windows)]
    use std::time::{Duration, Instant};

    #[cfg(windows)]
    use ::windows::Win32::System::Threading::{CREATE_NO_WINDOW, DETACHED_PROCESS};

    #[cfg(windows)]
    use super::{GroupOwner, MemberOutcome, PinnedMember, PortOutcome};
    #[cfg(windows)]
    use crate::testing::{is_ping, open_gate, spawn_gated_cmd, wait_until};

    /// 시험이 고른 끝내기 종료 코드 — 무리 통째 끝내기(1)와 다르기만 하면 된다.
    #[cfg(windows)]
    const EXIT_CODE: u32 = 0x7E57;

    #[cfg(windows)]
    fn new_group() -> (GroupOwner, GroupRef) {
        let owner = GroupOwner::new().expect("무리 만들기");
        let group = owner
            .downgrade()
            .expect("Windows 주인은 약한 손잡이를 낸다");
        (owner, group)
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
            .creation_flags(DETACHED_PROCESS.0)
            .spawn()
            .expect("ping.exe 기동")
    }

    #[cfg(windows)]
    fn pin_ours(group: &GroupRef, pid: u32, kill: bool) -> PinnedMember {
        group
            .pin_member(pid, kill)
            .expect("붙들기")
            .unwrap_or_else(|| panic!("PID {pid} 가 우리 무리의 멤버가 아니다"))
    }

    /// 붙든 멤버의 부모가 끝났나 — 적힌 부모 번호를 우리 무리에서 붙들 수 없다 · 붙든 부모가 끝났다 · 그 번호를
    /// 자기보다 늦게 태어난 것이 쥐었다. ★부모가 우리 무리에서 태어났다는 전제다★ — 무리 밖의 산 부모도 「끝남」
    /// 으로 읽힌다. 못 붙들면(권한 거부 등) 끝남으로 치지 않는다.
    #[cfg(windows)]
    fn parent_is_dead(group: &GroupRef, member: &PinnedMember) -> bool {
        let child = member.facts();
        assert!(
            child.ppid != 0 && child.create != 0,
            "부모 번호나 생성 시각을 못 읽었다: {child:?}"
        );
        match group.pin_member(child.ppid, false) {
            Ok(None) => true,
            Ok(Some(parent)) => {
                parent.exited().expect("끝났나") || parent.facts().create > child.create
            }
            Err(_) => false,
        }
    }

    /// 프로세스 하나를 지켜보는 핸들 — 끝나기를 기다리고 종료 코드를 읽는다. 쥐는 동안 그 프로세스 객체가 남는다.
    #[cfg(windows)]
    struct Watch(::windows::Win32::Foundation::HANDLE);

    #[cfg(windows)]
    impl Watch {
        fn open(pid: u32) -> Self {
            use ::windows::Win32::System::Threading::{
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
            use ::windows::Win32::Foundation::WAIT_OBJECT_0;
            use ::windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
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
                let _ = ::windows::Win32::Foundation::CloseHandle(self.0);
            }
        }
    }

    /// 손잡이는 무리를 붙들지 않는다 — 주인을 놓으면 그때부터 사라진 것으로 읽힌다.
    #[cfg(windows)]
    #[test]
    fn a_group_is_gone_once_its_job_is_dropped() {
        let (job, group) = new_group();
        assert!(!group.is_gone());
        drop(job);
        assert!(group.is_gone());
        assert!(group
            .pin_member(std::process::id(), false)
            .expect("붙들기")
            .is_none());
    }

    /// ② 무리 안 cmd 의 자식 ping 을 끝내기 권한으로 붙들어 끝낸다 — 붙든 핸들로 끝남을 기다리고, 종료 코드로 누가
    /// 끝냈는지 남는다. 끝난 뒤에도 쥐고 있어 그 번호는 명단 확인 동안 재사용되지 않는다.
    #[cfg(windows)]
    #[test]
    fn a_pinned_member_is_terminated_with_the_given_code() {
        let (job, group) = new_group();
        let mut root = spawn_gated_cmd("ping -n 30 127.0.0.1", CREATE_NO_WINDOW.0);
        job.adopt(root.id()).expect("무리 편입");
        open_gate(&mut root);

        let ping = wait_until("무리 안의 ping", || {
            group
                .member_pids()
                .ok()?
                .into_iter()
                .find(|&pid| is_ping(pid))
        });
        let watch = Watch::open(ping);
        let member = pin_ours(&group, ping, true);
        assert!(!member.exited().expect("끝났나"));

        let raw = member.terminate_raw(EXIT_CODE);
        assert_eq!(
            member.classify(raw).expect("끝내기"),
            MemberOutcome::Terminated
        );
        assert!(
            member
                .wait_exit(Duration::from_secs(5))
                .expect("끝나기 대기"),
            "5 초 안에 안 끝났다"
        );
        assert!(member.exited().expect("끝났나"));
        assert_eq!(watch.exit_code_within(Duration::ZERO), Some(EXIT_CODE));
        drop(watch);
        wait_until("명단에서 빠진 ping", || {
            (!group.member_pids().ok()?.contains(&ping)).then_some(())
        });
        drop(member);
        let _ = root.wait();
    }

    /// ③ 다른 무리의 산 프로세스는 붙들지 않는다(조사 §4 사고의 회귀망).
    #[cfg(windows)]
    #[test]
    fn a_process_in_another_job_is_not_ours() {
        let (_ours, group) = new_group();
        let other = GroupOwner::new().expect("남의 무리");
        let mut stranger = spawn_lone_ping();
        other.adopt(stranger.id()).expect("남의 무리 편입");

        for kill in [false, true] {
            assert!(
                group
                    .pin_member(stranger.id(), kill)
                    .expect("붙들기")
                    .is_none(),
                "남의 무리 멤버를 붙들었다(끝내기 권한 {kill})"
            );
        }
        assert!(stranger.try_wait().expect("상태").is_none(), "남이 죽었다");

        let _ = stranger.kill();
        let _ = stranger.wait();
    }

    /// ③ 중첩 변형 — 우리 무리가 부모 Job P 아래로 중첩돼도 P 에만 든 남은 우리 것이 아니다. Job 인자로 NULL 을
    /// 넘기면(「아무 Job 에나 들었나」) 여기서 빨개진다.
    #[cfg(windows)]
    #[test]
    fn a_process_only_in_the_enclosing_job_is_not_ours() {
        let parent = GroupOwner::new().expect("부모 무리");
        let (ours, group) = new_group();
        let mut member = spawn_lone_ping();
        parent.adopt(member.id()).expect("부모 무리 편입");
        ours.adopt(member.id())
            .expect("우리 무리 편입 — 부모 아래로 중첩");
        let mut stranger = spawn_lone_ping();
        parent.adopt(stranger.id()).expect("부모 무리에만 편입");

        assert!(
            group
                .pin_member(member.id(), true)
                .expect("붙들기")
                .is_some(),
            "중첩이 안 섰다 — 우리 멤버를 못 붙들었다"
        );
        assert!(
            group
                .pin_member(stranger.id(), true)
                .expect("붙들기")
                .is_none(),
            "부모 무리에만 든 남을 붙들었다"
        );
        assert!(stranger.try_wait().expect("상태").is_none(), "남이 죽었다");

        for child in [&mut member, &mut stranger] {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// ⑦ 손잡이를 거친 포트 — `start` 의 오류는 그대로 오고(붙지 않음), 받은 포트에서 붙일 때 이미 든 멤버가
    /// `Joined` 로 다시 알려진다. 붙이기 · 떼기의 OS 동작 자체는 `windows` 조각의 ⑦ 이 잰다.
    #[cfg(windows)]
    #[test]
    fn a_port_through_the_group_reports_the_members_already_in() {
        let (job, group) = new_group();
        let mut ping = spawn_lone_ping();
        job.adopt(ping.id()).expect("무리 편입");

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
        while port.next(Duration::from_millis(100)).expect("꺼내기")
            != PortOutcome::Joined(ping.id())
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

    /// ① 끝났지만 핸들이 붙든(좀비) 부모 — 무리 명단에서 빠지고 명단은 완전하다(좀비가 멤버 수에만 남으면 명단이
    /// 영영 불완전해 정리가 한 번도 돌지 못한다). 붙든 자식에서 보면 그 부모는 끝났다 — 좀비가 산 것으로 읽히면
    /// 잔여물을 놓친다.
    #[cfg(windows)]
    #[test]
    fn a_dead_parent_held_open_is_off_the_list_and_reads_dead() {
        let (job, group) = new_group();
        // C(ping)의 출력을 받는 파일 — C 가 첫 줄을 쓴 뒤에야 P 를 끝낸다. 기동을 마치지 않은 C 는 P 가 끝날 때
        // 함께 죽어(종료 코드 0xC000010A) 명단이 빈다(병렬 부하에서 11/60 · 끝낼 때 C 나이 1~4 ms 인 판 · 실측).
        // 첫 줄은 제 코드를 돌기 시작한 ping 만 쓴다.
        let ready =
            std::env::temp_dir().join(format!("t40-dead-parent-{}.txt", std::process::id()));
        let _ = std::fs::remove_file(&ready);
        // P 는 ping 을 띄운 뒤 둘째 `set /p` 에서 stdin 을 기다리며 산다 — 끝은 아래 kill 한 곳이다. 둘째 `set /p`
        // 가 없으면 `cmd /c` 가 `start /b` 직후 끝나 P 가 탐침 전에 이미 죽어 있다(실측 5/5).
        let mut parent = spawn_gated_cmd(
            &format!(
                "start \"\" /b ping -n 30 127.0.0.1 >\"{}\" & set /p _=",
                ready.display()
            ),
            0,
        );
        let parent_pid = parent.id();
        job.adopt(parent_pid).expect("무리 편입");
        let gate = open_gate_keeping_stdin(&mut parent);
        let (child_pid, child) = wait_until("P 가 띄운 ping", || {
            group
                .member_pids()
                .ok()?
                .into_iter()
                .filter(|&pid| is_ping(pid))
                .find_map(|pid| {
                    let member = group.pin_member(pid, false).ok()??;
                    (member.facts().ppid == parent_pid).then_some((pid, member))
                })
        });
        wait_until("ping 의 첫 줄", || {
            (std::fs::metadata(&ready).ok()?.len() > 0).then_some(())
        });
        assert!(
            parent.try_wait().expect("상태").is_none(),
            "P 가 탐침 전에 끝났다"
        );
        assert!(
            !parent_is_dead(&group, &child),
            "산 P 가 끝남으로 읽힌다 — 아래 단언이 뜻이 없다"
        );
        let _ = parent.kill();
        let _ = parent.wait();
        drop(gate);
        // `parent` 를 아직 쥐고 있다 — 그 핸들이 P 의 프로세스 객체를 붙든다.

        // 대기가 돌아온 직후에는 P 가 아직 명단에 있을 수 있다(`member_pids` 문서) — 빠지기만 기다린다. 완전함은
        // 기다리지 않는다: 부를 때마다 서야 한다.
        let members = wait_until(
            "명단에서 빠진 P(끝났지만 핸들이 붙든 것)",
            || {
                let members = group
                    .member_pids()
                    .expect("끝났지만 핸들이 붙든 멤버가 있어도 명단은 완전해야 한다");
                (!members.contains(&parent_pid)).then_some(members)
            },
        );
        assert!(
            members.contains(&child_pid),
            "산 C 가 명단에 없다: {members:?}"
        );
        assert!(
            parent_is_dead(&group, &child),
            "C 의 부모가 끝남으로 안 읽힌다"
        );
        drop(parent);
        job.terminate(1).expect("무리 끝내기");
        let _ = child.wait_exit(Duration::from_secs(5));
        let _ = std::fs::remove_file(&ready);
    }

    /// ⑤ 부모가 끝난 MSYS fork 자식이 우리 무리 멤버로 남는지 · 부모 끝남으로 읽히는지 · 끝나는지.
    /// ★무리 상속만 잰다 — 멈춤은 재현하지 않는다★. `PATH` 의 `bash` 는 WSL 일 수 있어 경로를 박는다.
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
            .creation_flags(CREATE_NO_WINDOW.0)
            .spawn()
            .expect("bash 기동");
        job.adopt(bash.id()).expect("무리 편입");
        open_gate(&mut bash);
        let status = bash.wait().expect("bash 종료");
        assert!(
            status.success(),
            "bash 가 {status:?} 로 끝났다 — 스크립트가 안 돌았다"
        );
        drop(bash);

        let orphans = wait_until("부모가 끝난 MSYS 프로세스", || {
            let found: Vec<(u32, PinnedMember)> = group
                .member_pids()
                .ok()?
                .into_iter()
                .filter_map(|pid| Some((pid, group.pin_member(pid, true).ok()??)))
                .filter(|(_, m)| {
                    m.facts()
                        .image
                        .to_ascii_lowercase()
                        .contains("\\usr\\bin\\")
                })
                .filter(|(_, m)| parent_is_dead(&group, m))
                .collect();
            (!found.is_empty()).then_some(found)
        });
        for (pid, orphan) in orphans {
            let watch = Watch::open(pid);
            let raw = orphan.terminate_raw(EXIT_CODE);
            assert_eq!(
                orphan.classify(raw).expect("끝내기"),
                MemberOutcome::Terminated
            );
            assert_eq!(
                watch.exit_code_within(Duration::from_secs(5)),
                Some(EXIT_CODE)
            );
            assert!(orphan.exited().expect("끝났나"));
        }
    }
}
