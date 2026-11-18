//! The **neutral handle** of the process group a path launched — member list · holding one member (facts · ending) · join notification
//! port · the path's retiring marker (read-only).
//!
//! It does not know what to end (ADR-0004) — that is the user's job. It asks the OS layer's weak group handle
//! (`engram_dashboard_platform::group::GroupRef`) and carries the answer into this crate's values · traits; OS branches live there
//! (ADR-0266), so this file has no production `cfg`. The traits test fakes plug into ([`Pinned`] · [`Births`]) and their trait objects are
//!   made only here (ADR-0266 decision 5).
//!
//! ★It does not hold the group★ — neither the handle nor the held member · port · marker holds it strongly (the call-time window:
//! `GroupRef` doc). If the path is gone: empty list, nobody can be held, port = [`GROUP_GONE`].
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

/// Facts read **once** with the held handle — they belong to that process object, not the number. Unreadable: `ppid` · `create` = 0,
///   strings = empty.
///
/// - `ppid` = parent number recorded at creation (that number may no longer be the same parent).
/// - `create` = creation time (FILETIME · 100 ns) — only for ordering two processes by creation.
/// - `image` = full executable path (Win32 form) · `cmdline` = the command line as is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ProcessFacts {
    pub ppid: u32,
    pub create: u64,
    pub image: String,
    pub cmdline: String,
}

/// The answer of [`Births::next`].
///
/// - `Joined(pid)` = a member newly joined. Only a number — it may have ended and been reused by now, so hold it right away for the
///   identity to stand.
/// - `Other` = another notification (ended, etc.) or a packet not this port's. `Timeout` = no notification during the wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PortEvent {
    Joined(u32),
    Other,
    Timeout,
}

/// A copy that **only reads** the path's retiring marker — the cell's owner (the path) makes it with [`Self::of`] and puts it on
///   [`ProcessGroup`].
/// ★No method sets it★ — only the cell's owner sets it; the reading side knows neither the manager nor the path's doors.
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

/// One process confirmed as a member of our group and held — while held its number is not reused and the facts remain after it ends. It
/// does not hold the group; dropping closes the handle.
///
/// Ending is split in two so the caller makes **only one** OS call inside a lock: `terminate_raw` = one ending call · `classify` = sorting
///   its raw result.
pub(crate) trait Pinned: Send + Sync {
    /// Has it ended now — does not wait. `Err` = failure of the wait query itself.
    fn exited(&self) -> io::Result<bool>;
    /// Wait until `d` for it to end — `true` if it ended. `Err` = failure of the wait query itself.
    fn wait_exit(&self, d: Duration) -> io::Result<bool>;
    fn facts(&self) -> &ProcessFacts;
    /// One ending call that gives `exit_code` — it does not wait for the target to end. One held without ending permission is always `Err`.
    fn terminate_raw(&self, exit_code: u32) -> io::Result<()>;
    /// Sort the raw result of [`Self::terminate_raw`] — even on failure, if it ended on its own meanwhile it is `Gone`.
    fn classify(&self, raw: io::Result<()>) -> io::Result<MemberKill>;
}

/// The group's join notification port — [`ProcessGroup::watch_births`] hands it to `start`. Shared by several threads; the port handle
/// closes when the last one goes. It does not hold the group. A notification is only a notice and delivery is not guaranteed (MS Learn) —
///   a missed join cannot be known here.
///
/// ★Detach first ([`ProcessGroup::unwatch_births`]), then release the last — never release it still attached★ (reason: the OS layer's
///   `BirthPort` doc).
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

    #[cfg(test)]
    pub(crate) fn detached(retiring: RetiringSignal) -> Self {
        Self {
            group: GroupRef::gone(),
            retiring,
        }
    }

    /// A complete list of member PIDs (including the root). `Ok(empty list)` if the path is gone · `Err` if incomplete.
    ///
    /// ★A member that just ended may remain briefly★ — liveness is seen from the held handle ([`Pinned::exited`]), not the list.
    pub(crate) fn member_pids(&self) -> io::Result<Vec<u32>> {
        self.group.member_pids()
    }

    /// Hold the process of that number **only if it is a member of our group** and read the facts once; liveness is not looked at.
    ///
    /// `kill` = also open ending permission. `Ok(None)` = no such process · outside our group · the path vanished · no group on this OS.
    /// `Err` = open failure (other than absence, e.g. permission denied) · membership query failure.
    pub(crate) fn pin(&self, pid: u32, kill: bool) -> io::Result<Option<Box<dyn Pinned>>> {
        Ok(self
            .group
            .pin_member(pid, kill)?
            .map(|member| Box::new(Member::from(member)) as Box<dyn Pinned>))
    }

    /// Make the join notification port, hand it to `start`, and attach the group to it **only if `start` is `Ok`** (an `Err` of `start` is
    /// returned as is, not attached).
    ///
    /// ★`start` must not block★ — it holds the group strongly while running, delaying group close if the path vanishes; it only launches
    ///   the taking-out thread.
    ///
    /// Members already in at attach time are also notified as `Joined`. ★Call once per group★ — a second call is not blocked here. `Err` kinds:
    /// [`GROUP_GONE`] = the path vanished (`start` not called) · `Unsupported` = no group on this OS (`start` not called) · others = the
    /// error of `start` · port creation · attaching.
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

    /// Is the group absent because the path vanished — looked at without bringing the group up (no side effects). Once true it stays true.
    ///   Always true on an OS with no group.
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

    /// A real group and the handle on it — used by the real-process test of claude residue cleanup. If the owner is released, the handle
    /// sees a vanished group. Nobody sets the retiring marker.
    #[cfg(windows)]
    pub(crate) fn new_group() -> (GroupOwner, ProcessGroup) {
        let owner = GroupOwner::new().expect("create group");
        let group = owner
            .downgrade()
            .expect("the Windows owner hands out a weak handle");
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
        owner.adopt(x.id()).expect("adopt into group");

        let member = group
            .pin(x.id(), true)
            .expect("hold on")
            .expect("a member of our group");
        let raw = member.terminate_raw(EXIT_CODE);
        assert_eq!(
            member.classify(raw).expect("finish"),
            MemberKill::Terminated
        );
        let status = x.wait().expect("wait for exit");
        assert_eq!(status.code(), Some(EXIT_CODE as i32));
    }
}
