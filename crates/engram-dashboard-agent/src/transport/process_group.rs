//! 통로가 띄운 프로세스 무리의 **중립 손잡이** — 멤버 명단 · 멤버 하나 붙들기(사실 · 끝내기) · 가입 알림 포트 ·
//! 통로의 물러남 표시(읽기 전용).
//!
//! 무엇을 끝낼지는 모른다(ADR-0004) — 고르기는 이것을 받아 쓰는 쪽의 몫이다. 여기는 OS 층의 약한 무리 손잡이
//! (`engram_dashboard_platform::group::GroupRef`)에 묻고, 그 답을 이 crate 의 값 · 트레이트로 옮겨 싣는다. OS 갈래는
//! 그 OS 층에 있다(ADR-0266) — 그래서 이 파일에는 운영 `cfg` 가 없다. 시험용 가짜가 끼우는 트레이트([`Pinned`] ·
//! [`Births`])와 그 트레이트 객체는 이 파일에서만 만든다(ADR-0266 결정 5).
//!
//! ★무리를 붙들지 않는다★ — 손잡이도, 내준 붙든 멤버 · 포트 · 표시도 무리를 강하게 쥐지 않는다(부르는 동안만
//! 쥐는 창은 `GroupRef` 문서). 통로가 사라졌으면 명단은 비고 아무도 못 붙들며 포트는 [`GROUP_GONE`] 이다.
// ADR-0262
// ADR-0266
// ADR-0275

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use engram_dashboard_platform::group::{
    BirthPort, GroupRef, MemberFacts, MemberOutcome, PinnedMember, PortOutcome,
};

pub(crate) use engram_dashboard_platform::group::GROUP_GONE;

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
/// 남는다. 무리를 붙들지 않는다. 버리면 핸들이 닫힌다.
///
/// 끝내기가 두 메서드로 갈린 것은 부르는 쪽이 자물쇠 안에서 OS 호출을 **하나만** 하게 하려는 것이다:
/// `terminate_raw` = 끝내기 호출 하나 · `classify` = 그 날것 결과 가르기.
pub(crate) trait Pinned: Send + Sync {
    /// 지금 끝났나 — 기다리지 않는다. `Err` = 대기 조회 자체의 실패.
    fn exited(&self) -> io::Result<bool>;
    /// `d` 까지 끝나기를 기다린다 — 끝났으면 `true`. `Err` = 대기 조회 자체의 실패.
    fn wait_exit(&self, d: Duration) -> io::Result<bool>;
    fn facts(&self) -> &ProcessFacts;
    /// `exit_code` 를 주는 끝내기 호출 하나 — 대상이 끝나기를 기다리지 않는다. 끝내기 권한 없이 붙든 것은 늘
    /// `Err` 다.
    fn terminate_raw(&self, exit_code: u32) -> io::Result<()>;
    /// [`Self::terminate_raw`] 의 날것 결과를 가른다 — 실패여도 그 사이 스스로 끝났으면 `Gone` 이다.
    fn classify(&self, raw: io::Result<()>) -> io::Result<MemberKill>;
}

/// 무리의 가입 알림 포트 — [`ProcessGroup::watch_births`] 가 `start` 에 넘긴다. 여러 스레드가 나눠 쥐고, 마지막
/// 것이 사라지면 포트 핸들이 닫힌다. 무리를 붙들지 않는다.
///
/// 알림은 통지일 뿐 배달이 보장되지 않는다(MS Learn) — 빠진 가입은 여기서 알 수 없다.
///
/// ★무리가 붙은 채 마지막 것을 놓지 않는다 — 먼저 떼고([`ProcessGroup::unwatch_births`]) 놓는다★(까닭 = OS 층
/// `BirthPort` 문서).
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

pub(crate) struct ProcessGroup {
    group: GroupRef,
    retiring: RetiringSignal,
}

impl ProcessGroup {
    pub(crate) fn new(group: GroupRef, retiring: RetiringSignal) -> Self {
        Self { group, retiring }
    }

    /// 무리가 이미 사라진 손잡이 — 모든 OS 에서 같은 모양이다.
    #[cfg(test)]
    pub(crate) fn detached(retiring: RetiringSignal) -> Self {
        Self {
            group: GroupRef::gone(),
            retiring,
        }
    }

    /// 완전한 멤버 PID 명단(뿌리 포함). 통로가 사라졌으면 `Ok(빈 목록)` · 불완전하면 `Err`.
    ///
    /// ★막 끝난 멤버가 잠깐 남을 수 있다★ — 살았는지는 명단이 아니라 붙든 핸들([`Pinned::exited`])로 본다.
    pub(crate) fn member_pids(&self) -> io::Result<Vec<u32>> {
        self.group.member_pids()
    }

    /// 그 번호의 프로세스를 **우리 무리의 멤버일 때만** 붙들고 사실을 한 번 읽는다. 살았는지는 보지 않는다.
    ///
    /// `kill` = 끝내기 권한도 연다. `Ok(None)` = 그 번호의 프로세스가 없다 · 우리 무리 밖이다 · 통로가 이미
    /// 사라졌다 · 이 OS 에는 무리가 없다. `Err` = 여는 실패(부재 말고 — 권한 거부 등) · 소속 조회 실패.
    pub(crate) fn pin(&self, pid: u32, kill: bool) -> io::Result<Option<Box<dyn Pinned>>> {
        Ok(self
            .group
            .pin_member(pid, kill)?
            .map(|member| Box::new(Member::from(member)) as Box<dyn Pinned>))
    }

    /// 가입 알림 포트를 만들어 `start` 에 넘기고, **`start` 가 `Ok` 일 때만** 무리를 그 포트에 붙인다. `start` 의
    /// `Err` 는 그대로 돌려준다(붙지 않았다).
    ///
    /// ★`start` 는 막히지 않아야 한다★ — 도는 동안 무리를 강하게 쥐어 통로가 사라져도 무리 닫기가 그만큼 늦는다.
    /// 그래서 `start` 는 꺼낼 스레드를 띄우기만 한다.
    ///
    /// 붙일 때 이미 든 멤버도 `Joined` 로 다시 알려진다. ★한 무리에 한 번만 부른다★ — 두 번째 부름을 여기서 막지
    /// 않는다(부르는 쪽 몫). `Err` 종류: [`GROUP_GONE`] = 통로가 이미 사라졌다(`start` 는 불리지 않았다) ·
    /// `Unsupported` = 이 OS 에는 무리가 없다(`start` 는 불리지 않았다) · 그 밖 = `start` 의 오류 · 포트 만들기 ·
    /// 붙이기의 OS 실패.
    pub(crate) fn watch_births(
        &self,
        start: impl FnOnce(Arc<dyn Births>) -> io::Result<()>,
    ) -> io::Result<()> {
        self.group.watch_births(|port| {
            let port: Arc<dyn Births> = port;
            start(port)
        })
    }

    /// 무리의 포트 연결을 뗀다 — 뗀 뒤 태어난 멤버는 알려지지 않는다. `Err` 종류는 [`Self::watch_births`] 와 같다.
    pub(crate) fn unwatch_births(&self) -> io::Result<()> {
        self.group.unwatch_births()
    }

    pub(crate) fn retiring(&self) -> RetiringSignal {
        self.retiring.clone()
    }

    /// 통로가 사라져 무리가 없나 — 무리를 올리지 않고 본다(부작용 없음). 한 번 참이면 계속 참이다. 이 OS 에는
    /// 무리가 없어 늘 참이다.
    pub(crate) fn is_gone(&self) -> bool {
        self.group.is_gone()
    }
}

/// OS 층의 붙든 멤버 + 이 crate 의 값으로 옮겨 실은 사실 — [`Pinned::facts`] 가 참조를 내주려고 함께 쥔다.
struct Member {
    pin: PinnedMember,
    facts: ProcessFacts,
}

impl From<PinnedMember> for Member {
    fn from(pin: PinnedMember) -> Self {
        let facts = ProcessFacts::from(pin.facts());
        Self { pin, facts }
    }
}

impl Pinned for Member {
    fn exited(&self) -> io::Result<bool> {
        self.pin.exited()
    }

    fn wait_exit(&self, d: Duration) -> io::Result<bool> {
        self.pin.wait_exit(d)
    }

    fn facts(&self) -> &ProcessFacts {
        &self.facts
    }

    fn terminate_raw(&self, exit_code: u32) -> io::Result<()> {
        self.pin.terminate_raw(exit_code)
    }

    fn classify(&self, raw: io::Result<()>) -> io::Result<MemberKill> {
        self.pin.classify(raw).map(MemberKill::from)
    }
}

impl Births for BirthPort {
    fn next(&self, wait: Duration) -> io::Result<PortEvent> {
        // 경로 호출은 고유 메서드를 먼저 고른다 — 이 trait 메서드로 되돌아오지 않는다.
        BirthPort::next(self, wait).map(PortEvent::from)
    }
}

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

impl From<PortOutcome> for PortEvent {
    fn from(outcome: PortOutcome) -> Self {
        match outcome {
            PortOutcome::Joined(pid) => Self::Joined(pid),
            PortOutcome::Other => Self::Other,
            PortOutcome::Timeout => Self::Timeout,
        }
    }
}

impl From<MemberOutcome> for MemberKill {
    fn from(outcome: MemberOutcome) -> Self {
        match outcome {
            MemberOutcome::Terminated => Self::Terminated,
            MemberOutcome::Gone => Self::Gone,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[cfg(windows)]
    use engram_dashboard_platform::group::GroupOwner;

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

    /// 실물 무리와 그 위의 손잡이 — claude 잔여물 정리의 실프로세스 시험이 쓴다. 주인을 놓으면 손잡이는 사라진
    /// 무리를 본다. 물러남 표시는 아무도 세우지 않는다.
    #[cfg(windows)]
    pub(crate) fn new_group() -> (GroupOwner, ProcessGroup) {
        let owner = GroupOwner::new().expect("무리 만들기");
        let group = owner
            .downgrade()
            .expect("Windows 주인은 약한 손잡이를 낸다");
        (
            owner,
            ProcessGroup::new(group, RetiringSignal::of(&Arc::default())),
        )
    }

    /// 손잡이로 붙든 멤버의 끝내기는 받은 종료 코드를 그대로 OS 에 넘긴다 — 어댑터가 다른 값을 넘기면 빨개진다.
    /// cmd 는 문을 열지 않아 stdin 의 한 줄을 기다리며 산다.
    #[cfg(windows)]
    #[test]
    fn a_pinned_member_ends_with_the_exit_code_it_was_given() {
        use engram_dashboard_platform::testing::spawn_gated_cmd;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const EXIT_CODE: u32 = 0x7E58;

        let (owner, group) = new_group();
        let mut x = spawn_gated_cmd("rem adapter-exit-code", CREATE_NO_WINDOW);
        owner.adopt(x.id()).expect("무리 편입");

        let member = group
            .pin(x.id(), true)
            .expect("붙들기")
            .expect("우리 무리의 멤버");
        let raw = member.terminate_raw(EXIT_CODE);
        assert_eq!(
            member.classify(raw).expect("끝내기"),
            MemberKill::Terminated
        );
        let status = x.wait().expect("종료 대기");
        assert_eq!(status.code(), Some(EXIT_CODE as i32));
    }
}
