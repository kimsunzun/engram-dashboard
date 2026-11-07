//! The **neutral handle** of the process group a path launched — member list · holding one member (facts · ending) · join notification port ·
//! the path's retiring marker (read-only).
//!
//! It does not know what to end (ADR-0004) — choosing is the job of the side that takes and uses this. Here we ask the OS layer's weak group handle
//! (`engram_dashboard_platform::group::GroupRef`) and carry its answer over into this crate's values · traits. The OS branches are
//! in that OS layer (ADR-0266) — so this file has no production `cfg`. The traits the test fakes plug into ([`Pinned`] ·
//! [`Births`]) and their trait objects are made only in this file (ADR-0266 decision 5).
//!
//! ★It does not hold the group★ — neither the handle nor the handed-out held member · port · marker holds the group strongly (the window held only
//! during a call is in the `GroupRef` doc). If the path is gone, the list is empty, nobody can be held, and the port is [`GROUP_GONE`].
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

/// Facts read **once** with the held handle — they belong to that process object, not to the number. Fields that could not be read are `ppid` · `create` =
/// 0 · strings = the empty string.
///
/// - `ppid` = the parent number recorded at creation — there is no guarantee that number is still the same parent now.
/// - `create` = creation time (FILETIME · 100 ns) — meaningful only for comparing the creation order of two processes.
/// - `image` = the full executable path (Win32 form) · `cmdline` = the command line as is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ProcessFacts {
    pub ppid: u32,
    pub create: u64,
    pub image: String,
    pub cmdline: String,
}

/// The answer of [`Births::next`].
///
/// - `Joined(pid)` = a member newly joined. It is only a number, so it may have already ended and been reused by the time it is taken out — it must be held
///   right away for the identity to stand.
/// - `Other` = another notification (ended, etc.) or a packet that is not this port's.
/// - `Timeout` = there was no notification during the wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PortEvent {
    Joined(u32),
    Other,
    Timeout,
}

/// A copy that **only reads** the path's retiring marker — the cell's owner (the path) makes it with [`Self::of`] and puts it on [`ProcessGroup`].
/// ★There is no method that sets it★ — only the cell's owner sets it, and the reading side sees only this, knowing neither the manager nor the path's doors.
#[derive(Debug, Clone)]
pub(crate) struct RetiringSignal(Arc<AtomicBool>);

impl RetiringSignal {
    /// Share the cell — only the cell's owner calls it. The owner sets it with `Release`.
    pub(crate) fn of(flag: &Arc<AtomicBool>) -> Self {
        Self(Arc::clone(flag))
    }

    pub(crate) fn is_set(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// One process confirmed as a member of our group and held — while held, its number is not reused, and the facts
/// remain even after it ends. It does not hold the group. If dropped, the handle is closed.
///
/// Ending is split into two methods so the caller makes **only one** OS call inside a lock:
/// `terminate_raw` = one ending call · `classify` = sorting that raw result.
pub(crate) trait Pinned: Send + Sync {
    /// Has it ended now — does not wait. `Err` = failure of the wait query itself.
    fn exited(&self) -> io::Result<bool>;
    /// Wait until `d` for it to end — `true` if it ended. `Err` = failure of the wait query itself.
    fn wait_exit(&self, d: Duration) -> io::Result<bool>;
    fn facts(&self) -> &ProcessFacts;
    /// One ending call that gives `exit_code` — it does not wait for the target to end. One held without ending permission is always
    /// `Err`.
    fn terminate_raw(&self, exit_code: u32) -> io::Result<()>;
    /// Sort the raw result of [`Self::terminate_raw`] — even on failure, if it ended on its own meanwhile it is `Gone`.
    fn classify(&self, raw: io::Result<()>) -> io::Result<MemberKill>;
}

/// The group's join notification port — [`ProcessGroup::watch_births`] hands it to `start`. Several threads share it, and when the last
/// one disappears the port handle is closed. It does not hold the group.
///
/// A notification is only a notice and delivery is not guaranteed (MS Learn) — a missed join cannot be known here.
///
/// ★Do not release the last one with the group still attached — detach first ([`ProcessGroup::unwatch_births`]) and then release★ (reason = the OS layer's
/// `BirthPort` doc).
pub(crate) trait Births: Send + Sync {
    /// Wait for one notification up to `wait` and take it out. `Err` = failure of taking out itself (the port closed, etc.).
    fn next(&self, wait: Duration) -> io::Result<PortEvent>;
}

/// `Gone` = it ended on its own after being held and before ending.
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

    /// A handle whose group has already vanished — the same shape on every OS.
    #[cfg(test)]
    pub(crate) fn detached(retiring: RetiringSignal) -> Self {
        Self {
            group: GroupRef::gone(),
            retiring,
        }
    }

    /// A complete list of member PIDs (including the root). `Ok(빈 목록)` if the path is gone · `Err` if incomplete.
    ///
    /// ★A member that just ended may remain briefly★ — whether it is alive is seen not from the list but from the held handle ([`Pinned::exited`]).
    pub(crate) fn member_pids(&self) -> io::Result<Vec<u32>> {
        self.group.member_pids()
    }

    /// Hold the process of that number **only if it is a member of our group** and read the facts once. Whether it is alive is not looked at.
    ///
    /// `kill` = also opens ending permission. `Ok(None)` = there is no process with that number · it is outside our group · the path has already
    /// vanished · this OS has no group. `Err` = open failure (other than absence — permission denied, etc.) · membership query failure.
    pub(crate) fn pin(&self, pid: u32, kill: bool) -> io::Result<Option<Box<dyn Pinned>>> {
        Ok(self
            .group
            .pin_member(pid, kill)?
            .map(|member| Box::new(Member::from(member)) as Box<dyn Pinned>))
    }

    /// Make the join notification port and hand it to `start`, and attach the group to that port **only if `start` is `Ok`**. The `Err` of `start`
    /// is returned as is (it was not attached).
    ///
    /// ★`start` must not block★ — while it runs it holds the group strongly, so even if the path vanishes, closing the group is delayed by that much.
    /// So `start` only launches the thread that takes out.
    ///
    /// Members already in at attach time are also notified again as `Joined`. ★Call it only once per group★ — a second call is not blocked here
    /// (the caller's job). `Err` kinds: [`GROUP_GONE`] = the path has already vanished (`start` was not called) ·
    /// `Unsupported` = this OS has no group (`start` was not called) · others = the error of `start` · port creation ·
    /// the OS failure of attaching.
    pub(crate) fn watch_births(
        &self,
        start: impl FnOnce(Arc<dyn Births>) -> io::Result<()>,
    ) -> io::Result<()> {
        self.group.watch_births(|port| {
            let port: Arc<dyn Births> = port;
            start(port)
        })
    }

    /// Detach the group's port connection — members born after detaching are not notified. `Err` kinds are the same as [`Self::watch_births`].
    pub(crate) fn unwatch_births(&self) -> io::Result<()> {
        self.group.unwatch_births()
    }

    pub(crate) fn retiring(&self) -> RetiringSignal {
        self.retiring.clone()
    }

    /// Is the group absent because the path vanished — looked at without bringing the group up (no side effects). Once true it stays true. On this OS
    /// there is no group so it is always true.
    pub(crate) fn is_gone(&self) -> bool {
        self.group.is_gone()
    }
}

/// The OS layer's held member + the facts carried over into this crate's values — held together so [`Pinned::facts`] can hand out a reference.
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
        // A path call picks the inherent method first — it does not come back to this trait method.
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

    /// The handed-out marker follows what the cell's owner set — passing through the group handle or cloning, it is the same cell.
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

    /// A real group and the handle on it — used by the real-process test of claude residue cleanup. If the owner is released, the handle sees a vanished
    /// group. Nobody sets the retiring marker.
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

    /// Ending a member held by the handle passes the received exit code to the OS as is — it goes red if the adapter passes another value.
    /// cmd does not open the door so it lives waiting for one line of stdin.
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
