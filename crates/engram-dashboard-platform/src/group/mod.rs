//! The OS layer for processes tied into one group — the strong owner [`GroupOwner`], the weak handle [`GroupRef`], and the concrete types
//!   the handle hands out (pinned member [`PinnedMember`], join-notification port
//! [`BirthPort`], value [`MemberFacts`], [`PortOutcome`], [`MemberOutcome`]). On Windows it is a Job Object (the private `windows` part;
//!   this module moves that type into its own public types); on other OSes a shell that ties nothing.
//!
//! It does not know what to end (ADR-0266 decision 3) — choosing or backing off belongs to the caller's adapter; here it only asks the OS
//!   whether a number is a group member, pins it, reads facts through that handle and ends it.
//! The exit code when ending is supplied by the caller (ADR-0275 decision 12). No trait objects in the public API — a test fake is the
//!   caller's small trait implemented on this concrete type (ADR-0266 decision 5).
//!
//! ★Neither logging nor locks★ — [`PinnedMember::terminate_raw`], [`PinnedMember::classify`] and [`GroupRef::is_gone`] are called by the
//!   caller holding a lock, and the OS calls in that lock are bundled into a single terminate
//! (ADR-0262 decision 8). Do not let this crate's `tracing` allowance (ADR-0266 decision 9) spread into this module.
// ADR-0262, ADR-0266, ADR-0275

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
pub(crate) use self::windows::resume_suspended_process;

/// The `Err` kind [`GroupRef::watch_births`] and [`GroupRef::unwatch_births`] return when the owner is already gone and there is no group
///   — not an OS failure, so kept separate; OS failures in creating, attaching or
/// detaching the port are `Other` (but a `start` that returns this kind cannot be told apart).
pub const GROUP_GONE: io::ErrorKind = io::ErrorKind::NotConnected;

/// The strong owner of a group — while alive the group exists. Dropping it on Windows closes the Job handle and the OS ends the remaining
///   members (`KILL_ON_JOB_CLOSE`) — if a [`GroupRef`] is mid-call the handle closes when that call ends.
/// ★Not `Clone`★ — the type enforces only "one strong owner per group"; who that owner is (launch path, tree handle) is kept by the
///   arrangement (ADR-0275 decision 11). Outward it hands out only [`Self::downgrade`]'s weak handle.
/// Outside Windows it ties nothing — `new`, `adopt`, `terminate` are `Ok`, `active_processes` is `Unsupported`, `downgrade` is `None`.
// ADR-0266, ADR-0275
pub struct GroupOwner {
    #[cfg(windows)]
    job: Arc<windows::JobObjectHandle>,
}

impl GroupOwner {
    /// Windows = creates an anonymous Job with `KILL_ON_JOB_CLOSE`. `Err` = an OS failure in creating or configuring it.
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

    /// Puts the process with that number into the group. ★Putting it in after launching leaves children launched before as outside the
    ///   group★ — close it by launching suspended, putting in, then waking ([`crate::spawn::prepare_tree_root`], [`crate::spawn::TreeRoot`]).
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

    /// Ends all members with `exit_code` — it only starts the ending and returns. Whether everything has ended is checked with
    /// [`Self::active_processes`].
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

    /// Number of processes in the group that have not yet ended. Outside Windows = `Unsupported`.
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

    /// A weak handle that does not hold the group. `None` = this OS has no means of tying a group together (outside Windows).
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

/// The weak handle of a group — member list, pinning one member (facts, ending), join-notification port, "is it gone". ★Holds the group
///   weakly★ — strongly would leave the Job handle unclosed after the owner is gone, delaying
/// `KILL_ON_JOB_CLOSE`. It upgrades only for each call ([`Self::is_gone`] does not even upgrade); the longest window is while
///   [`Self::watch_births`] calls `start` (so `start` must not block). If the owner is gone: empty list, nothing pinnable,
/// port [`GROUP_GONE`]. Pinned members and ports it hands out do not hold the group. Outside Windows it answers like a handle whose owner
///   is gone — only the port's `Err` kind is `Unsupported`.
// ADR-0266, ADR-0275
pub struct GroupRef {
    #[cfg(windows)]
    job: Weak<windows::JobObjectHandle>,
}

impl GroupRef {
    /// A handle that never had an owner — the same shape on every OS.
    #[cfg(any(test, feature = "test-support"))]
    pub fn gone() -> Self {
        Self {
            #[cfg(windows)]
            job: Weak::new(),
        }
    }

    /// The complete list of member PIDs (root and nested groups' members included; order meaningless). Owner gone → `Ok(empty)`; list not
    ///   complete within the retry limit → `Err` (a missing member is not read as "outside the group").
    /// ★A member that has just ended may linger briefly★ — it may leave the list later than the ending (the pinned-handle wait returning;
    ///   CI measurement 2026-10-01); judge liveness by the pinned handle ([`PinnedMember::exited`]), not the list.
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

    /// Pins the process with that number **only when it is a member of this group** and reads the facts once; liveness is not checked.
    ///   `kill` = also opens the terminate right. `Ok(None)` = no such process, outside this group (including one
    /// only in an outer Job wrapping this group), owner gone, or no group on this OS. `Err` = open failure other than absence (access
    ///   denied …) or membership query failure.
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

    /// Creates the join-notification port, passes it to `start`, and attaches the group **only when `start` is `Ok`** — by the time
    ///   notifications can pile up the taking side holds the port. `start`'s `Err` is returned as is (nothing attached).
    /// ★While `start` runs it holds the group strongly (upgraded `Arc`)★ — if the owner goes away meanwhile the Job handle does not close
    ///   and `KILL_ON_JOB_CLOSE` is delayed as long as `start` blocks, so `start` only spawns the notification-taking
    /// thread and does not block. Members already inside are announced again as `Joined` on attaching; notifications for a member changing
    ///   state during attaching may be missed (MS Learn). ★Call once per group★ (a second call is not
    /// blocked here). `Err` kinds: [`GROUP_GONE`] = owner gone (`start` not called) · `Unsupported` = no group on this OS (`start` not
    ///   called) · anything else = `start`'s error or an OS failure creating the port/attaching.
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

    /// Detaches the group's port connection — members born after detaching are not announced. `Err` kinds as [`Self::watch_births`].
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

    /// Is the owner gone so there is no group — looks without upgrading (no side effects, does not delay closing the Job handle). Once
    ///   true it stays true; always true on an OS without a group.
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

/// Facts read **once** through the pinned handle — they belong to that process object, not the number. Unreadable fields: `ppid` and
///   `create` = 0, strings empty. `ppid` = the parent number recorded at creation (no guarantee it is still
/// the same parent); `create` = creation time (FILETIME, 100 ns) — meaningful only for comparing creation order; `image` = full executable
///   path (Win32 form); `cmdline` = the command line as is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemberFacts {
    pub ppid: u32,
    pub create: u64,
    pub image: String,
    pub cmdline: String,
}

/// The answer of [`BirthPort::next`]. `Joined(pid)` = a member newly joined — only a number, so by the time it is taken out it may have
///   ended and been reused; pin it right away for the identity to stand. `Other` = any other notification
/// (ending etc.) or a packet not of this port. `Timeout` = nothing arrived while waiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortOutcome {
    Joined(u32),
    Other,
    Timeout,
}

/// The answer of [`PinnedMember::classify`]. `Gone` = it ended by itself after being pinned and before the terminate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberOutcome {
    Terminated,
    Gone,
}

/// One process confirmed as a member of this group and pinned — while held that number is not reused and the facts remain after it ends;
///   it does not hold the group; drop closes the handle; only [`GroupRef::pin_member`] creates it.
/// Terminating is two methods so the caller makes **one** OS call inside a lock: [`Self::terminate_raw`] = one terminate call ·
///   [`Self::classify`] = sorting the raw result.
#[derive(Debug)]
pub struct PinnedMember {
    #[cfg(windows)]
    os: windows::PinnedMember,
    // It is never created outside Windows — the method bodies say so with an empty match.
    #[cfg(not(windows))]
    os: std::convert::Infallible,
    facts: MemberFacts,
}

impl PinnedMember {
    /// Has it ended now — does not wait. `Err` = failure of the wait query itself.
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

    /// Waits up to `limit` for it to end — `true` if it ended. `Err` = failure of the wait query itself.
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

    /// Makes **only one** terminate call with `exit_code` on that pinned handle; does not wait for the end. Returns the raw result;
    ///   sorting is [`Self::classify`]. A pinned-without-terminate-right (`kill = false`) member is always `Err`;
    /// failure is also an error that does not use the heap.
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

    /// Sorts the raw result of [`Self::terminate_raw`] — on failure, if it ended by itself meanwhile it is `Gone`. The handle is not
    ///   closed (drop closes it).
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

/// The group's join-notification port — [`GroupRef::watch_births`] passes it to `start`; threads share it through `Arc` and the last drop
///   closes the handle; it does not hold the group. Notifications are only notices and delivery is not
/// guaranteed (MS Learn) — a missed join cannot be known here. ★Do not let go of the last one while the group is attached — detach first
///   ([`GroupRef::unwatch_births`])★: the attached group holds the port as its own kernel reference, so the port
/// survives a handle close and notifications pile up unread (not in MS Learn, not measured — detaching first loses nothing even if wrong).
#[derive(Debug)]
pub struct BirthPort {
    // The inner `Arc` is shared with the share the part's `watch_births` holds until it finishes attaching — a handle closed before that
    //   could be reused and someone else's handle attached.
    #[cfg(windows)]
    os: Arc<windows::BirthPort>,
    #[cfg(not(windows))]
    os: std::convert::Infallible,
}

impl BirthPort {
    /// Waits up to `wait` for one notification and takes it out. `Err` = failure of taking it out itself (port closed etc.).
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
    io::Error::new(GROUP_GONE, "the group's owner is already gone")
}

#[cfg(not(windows))]
fn no_group_here() -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, "this OS has no process groups")
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::{GroupRef, GROUP_GONE};

    /// The answer given by a handle with no owner — no members, nothing pinnable, no port (`start` not called); only the kind differs per OS.
    #[test]
    fn a_group_without_a_live_job_lists_nothing_and_owns_nobody() {
        let group = GroupRef::gone();
        assert!(group.is_gone());
        assert_eq!(group.member_pids().expect("list"), Vec::<u32>::new());
        for kill in [false, true] {
            assert!(group
                .pin_member(std::process::id(), kill)
                .expect("pin")
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
        assert_eq!(
            watched
                .expect_err("attached although there is no group")
                .kind(),
            refused
        );
        assert!(!started, "launched the taker although there is no group");
        assert_eq!(
            group
                .unwatch_births()
                .expect_err("detached although there is no group")
                .kind(),
            refused
        );
    }

    // ── real processes (Windows) — each test launches only a few, one after another: do not launch them in a burst (local parallel limit `--test-threads=4` — `.claude/skill-bindings/qa.md` 「모듈별 명령」) ── // hangul-ok

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

    /// The terminate exit code chosen by the test — only needs to differ from terminating the whole group (1).
    #[cfg(windows)]
    const EXIT_CODE: u32 = 0x7E57;

    #[cfg(windows)]
    fn new_group() -> (GroupOwner, GroupRef) {
        let owner = GroupOwner::new().expect("create group");
        let group = owner
            .downgrade()
            .expect("the Windows owner hands out a weak handle");
        (owner, group)
    }

    /// Like [`open_gate`] but returns stdin without closing it — if `line` reads stdin again (`set /p`), it waits there as long as the
    ///   returned end is held.
    #[cfg(windows)]
    fn open_gate_keeping_stdin(child: &mut Child) -> ChildStdin {
        let mut stdin = child.stdin.take().expect("stdin pipe");
        stdin.write_all(b"go\r\n").expect("open the gate");
        stdin
    }

    /// A single `ping` that runs without a console — it does not drag in a console host.
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
            .expect("launch ping.exe")
    }

    #[cfg(windows)]
    fn pin_ours(group: &GroupRef, pid: u32, kill: bool) -> PinnedMember {
        group
            .pin_member(pid, kill)
            .expect("pin")
            .unwrap_or_else(|| panic!("PID {pid} is not a member of our group"))
    }

    /// Has the pinned member's parent ended — the recorded parent number cannot be pinned in our group, the pinned parent has ended, or a
    ///   process born later than the member holds that number. ★Assumes the parent was born in our group★ —
    /// a live parent outside the group also reads as "ended". If it cannot be pinned (access denied etc.), it does not count as ended.
    #[cfg(windows)]
    fn parent_is_dead(group: &GroupRef, member: &PinnedMember) -> bool {
        let child = member.facts();
        assert!(
            child.ppid != 0 && child.create != 0,
            "could not read the parent number or creation time: {child:?}"
        );
        match group.pin_member(child.ppid, false) {
            Ok(None) => true,
            Ok(Some(parent)) => {
                parent.exited().expect("has it ended") || parent.facts().create > child.create
            }
            Err(_) => false,
        }
    }

    /// A handle that watches one process — waits for it to end and reads the exit code. While it is held that process object remains.
    #[cfg(windows)]
    struct Watch(::windows::Win32::Foundation::HANDLE);

    #[cfg(windows)]
    impl Watch {
        fn open(pid: u32) -> Self {
            use ::windows::Win32::System::Threading::{
                OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
            };
            // SAFETY: opens only the query and wait rights.
            let handle = unsafe {
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                    false,
                    pid,
                )
            }
            .expect("handle to watch");
            Self(handle)
        }

        fn exit_code_within(&self, limit: Duration) -> Option<u32> {
            use ::windows::Win32::Foundation::WAIT_OBJECT_0;
            use ::windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
            // SAFETY: valid handle · finite wait.
            if unsafe { WaitForSingleObject(self.0, limit.as_millis() as u32) } != WAIT_OBJECT_0 {
                return None;
            }
            let mut code = 0u32;
            // SAFETY: valid handle plus stack output.
            unsafe { GetExitCodeProcess(self.0, &mut code) }.ok()?;
            Some(code)
        }
    }

    #[cfg(windows)]
    impl Drop for Watch {
        fn drop(&mut self) {
            // SAFETY: closes the handle that open opened, exactly once.
            unsafe {
                let _ = ::windows::Win32::Foundation::CloseHandle(self.0);
            }
        }
    }

    /// The handle does not hold the group — once the owner is let go it reads as gone from then on.
    #[cfg(windows)]
    #[test]
    fn a_group_is_gone_once_its_job_is_dropped() {
        let (job, group) = new_group();
        assert!(!group.is_gone());
        drop(job);
        assert!(group.is_gone());
        assert!(group
            .pin_member(std::process::id(), false)
            .expect("pin")
            .is_none());
    }

    /// ② Pin the child ping of cmd in the group with the terminate right and end it — wait for the ending on the pinned handle; the exit
    ///   code shows who ended it. Still held after ending, so that number is not reused during the list check.
    #[cfg(windows)]
    #[test]
    fn a_pinned_member_is_terminated_with_the_given_code() {
        let (job, group) = new_group();
        let mut root = spawn_gated_cmd("ping -n 30 127.0.0.1", CREATE_NO_WINDOW.0);
        job.adopt(root.id()).expect("join the group");
        open_gate(&mut root);

        let ping = wait_until("ping inside the group", || {
            group
                .member_pids()
                .ok()?
                .into_iter()
                .find(|&pid| is_ping(pid))
        });
        let watch = Watch::open(ping);
        let member = pin_ours(&group, ping, true);
        assert!(!member.exited().expect("has it ended"));

        let raw = member.terminate_raw(EXIT_CODE);
        assert_eq!(
            member.classify(raw).expect("end"),
            MemberOutcome::Terminated
        );
        assert!(
            member
                .wait_exit(Duration::from_secs(5))
                .expect("wait for exit"),
            "did not end within 5 seconds"
        );
        assert!(member.exited().expect("has it ended"));
        assert_eq!(watch.exit_code_within(Duration::ZERO), Some(EXIT_CODE));
        drop(watch);
        wait_until("ping gone from the list", || {
            (!group.member_pids().ok()?.contains(&ping)).then_some(())
        });
        drop(member);
        let _ = root.wait();
    }

    /// ③ A live process of another group is not pinned (regression net for the incident in survey §4).
    #[cfg(windows)]
    #[test]
    fn a_process_in_another_job_is_not_ours() {
        let (_ours, group) = new_group();
        let other = GroupOwner::new().expect("someone else's group");
        let mut stranger = spawn_lone_ping();
        other
            .adopt(stranger.id())
            .expect("join someone else's group");

        for kill in [false, true] {
            assert!(
                group
                    .pin_member(stranger.id(), kill)
                    .expect("pin")
                    .is_none(),
                "pinned a member of someone else's group (termination right {kill})"
            );
        }
        assert!(
            stranger.try_wait().expect("status").is_none(),
            "someone else died"
        );

        let _ = stranger.kill();
        let _ = stranger.wait();
    }

    /// ③ Nested variant — even if our group is nested under a parent Job P, a process only in P is someone else's. Passing NULL as the Job
    ///   argument ("is it in any Job at all") makes this go red here.
    #[cfg(windows)]
    #[test]
    fn a_process_only_in_the_enclosing_job_is_not_ours() {
        let parent = GroupOwner::new().expect("parent group");
        let (ours, group) = new_group();
        let mut member = spawn_lone_ping();
        parent.adopt(member.id()).expect("join the parent group");
        ours.adopt(member.id())
            .expect("join our group — nested under the parent");
        let mut stranger = spawn_lone_ping();
        parent
            .adopt(stranger.id())
            .expect("join only the parent group");

        assert!(
            group.pin_member(member.id(), true).expect("pin").is_some(),
            "nesting did not happen — could not pin our member"
        );
        assert!(
            group
                .pin_member(stranger.id(), true)
                .expect("pin")
                .is_none(),
            "pinned someone else that is only in the parent group"
        );
        assert!(
            stranger.try_wait().expect("status").is_none(),
            "someone else died"
        );

        for child in [&mut member, &mut stranger] {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// ⑦ A port obtained through the handle — `start`'s error returns as is (not attached), and attaching from the received port
    ///   re-announces the members already inside as `Joined`; attach/detach OS behavior itself is measured by ⑦ in the `windows` part.
    #[cfg(windows)]
    #[test]
    fn a_port_through_the_group_reports_the_members_already_in() {
        let (job, group) = new_group();
        let mut ping = spawn_lone_ping();
        job.adopt(ping.id()).expect("join the group");

        let refused = group.watch_births(|_| Err(io::Error::other("mimicked launch failure")));
        assert_eq!(
            refused
                .expect_err("start's failure must come through as is")
                .to_string(),
            "mimicked launch failure"
        );

        let mut held = None;
        group
            .watch_births(|port| {
                held = Some(port);
                Ok(())
            })
            .expect("attach");
        let port = held.expect("start received the port");
        let deadline = Instant::now() + Duration::from_secs(10);
        while port.next(Duration::from_millis(100)).expect("take out")
            != PortOutcome::Joined(ping.id())
        {
            assert!(
                Instant::now() < deadline,
                "did not see ping's echoed notification within 10 seconds"
            );
        }
        group.unwatch_births().expect("detach");

        let _ = ping.kill();
        let _ = ping.wait();
    }

    /// ① A parent that has ended but whose handle holds it (zombie) — it drops out of the group list and the list is complete (a zombie
    ///   left only in the member count would keep the list incomplete forever and cleanup would never run once).
    /// From the pinned child's view that parent has ended — read as alive, leftovers are missed.
    #[cfg(windows)]
    #[test]
    fn a_dead_parent_held_open_is_off_the_list_and_reads_dead() {
        let (job, group) = new_group();
        // The file receiving C's (ping) output — P is ended only after C writes its first line (a C still starting dies when P ends, exit
        //   code 0xC000010A, and the list comes up empty: 11/60 under parallel load, C's age at ending 1–4 ms; measured).
        // The first line is written only by a ping that has started running its own code.
        let ready =
            std::env::temp_dir().join(format!("t40-dead-parent-{}.txt", std::process::id()));
        let _ = std::fs::remove_file(&ready);
        // P launches the ping and lives waiting on stdin at the second `set /p` — it ends at the single kill below. Without it `cmd /c`
        //   ends right after `start /b` and P is dead before the probe (measured 5/5).
        let mut parent = spawn_gated_cmd(
            &format!(
                "start \"\" /b ping -n 30 127.0.0.1 >\"{}\" & set /p _=",
                ready.display()
            ),
            0,
        );
        let parent_pid = parent.id();
        job.adopt(parent_pid).expect("join the group");
        let gate = open_gate_keeping_stdin(&mut parent);
        let (child_pid, child) = wait_until("the ping P launched", || {
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
        wait_until("ping's first line", || {
            (std::fs::metadata(&ready).ok()?.len() > 0).then_some(())
        });
        assert!(
            parent.try_wait().expect("status").is_none(),
            "P ended before the probe"
        );
        assert!(
            !parent_is_dead(&group, &child),
            "live P reads as ended — the assertions below mean nothing"
        );
        let _ = parent.kill();
        let _ = parent.wait();
        drop(gate);
        // `parent` is still held — that handle pins P's process object.

        // Right after the wait returns, P may still be in the list (`member_pids` docs) — wait only for it to drop out, not for
        //   completeness (that must hold on every call).
        let members = wait_until("P gone from the list (ended but held by a handle)", || {
            let members = group.member_pids().expect(
                "the list must be complete even with a member that ended but is held by a handle",
            );
            (!members.contains(&parent_pid)).then_some(members)
        });
        assert!(
            members.contains(&child_pid),
            "live C is not in the list: {members:?}"
        );
        assert!(
            parent_is_dead(&group, &child),
            "C's parent does not read as ended"
        );
        drop(parent);
        job.terminate(1).expect("end the group");
        let _ = child.wait_exit(Duration::from_secs(5));
        let _ = std::fs::remove_file(&ready);
    }

    /// ⑤ Whether an MSYS fork child whose parent has ended stays a member of our group, reads as parent-ended, and ends. ★Measures group
    ///   inheritance only — it does not reproduce the hang★. `bash` on `PATH` may be WSL, so the path is hard-coded.
    #[cfg(windows)]
    #[test]
    #[ignore = "needs Git Bash (C:\\Program Files\\Git\\usr\\bin\\bash.exe)"]
    fn an_orphaned_msys_fork_child_stays_in_our_job_and_ends() {
        use std::os::windows::process::CommandExt;
        const BASH: &str = r"C:\Program Files\Git\usr\bin\bash.exe";
        assert!(
            std::path::Path::new(BASH).exists(),
            "{BASH} is missing — the bash on PATH is not used instead"
        );
        let (job, group) = new_group();
        // `sleep` by absolute path — `usr\bin\bash.exe` started without the Git launcher (`Git\bin\bash.exe`) has no `/usr/bin` in `PATH`,
        //   so a bare `sleep` ends 127 (measured).
        let mut bash = Command::new(BASH)
            .args(["-c", "read _; (/usr/bin/sleep 30 &)"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW.0)
            .spawn()
            .expect("launch bash");
        job.adopt(bash.id()).expect("join the group");
        open_gate(&mut bash);
        let status = bash.wait().expect("bash exit");
        assert!(
            status.success(),
            "bash ended with {status:?} — the script did not run"
        );
        drop(bash);

        let orphans = wait_until("MSYS processes whose parent ended", || {
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
                orphan.classify(raw).expect("end"),
                MemberOutcome::Terminated
            );
            assert_eq!(
                watch.exit_code_within(Duration::from_secs(5)),
                Some(EXIT_CODE)
            );
            assert!(orphan.exited().expect("has it ended"));
        }
    }
}
