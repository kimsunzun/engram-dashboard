//! The OS layer for processes tied into one group — the strong owner [`GroupOwner`], the weak handle [`GroupRef`], and the
//! concrete types the handle hands out (pinned member [`PinnedMember`], join-notification port [`BirthPort`], value [`MemberFacts`], [`PortOutcome`],
//! [`MemberOutcome`]). On Windows it is a Job Object (the private `windows` part — this module moves
//! that type into its own public types), and on other OSes it is a shell that ties nothing together (see each method's docs).
//!
//! It does not know what to end (ADR-0266 decision 3) — meanings such as choosing or backing off belong to the caller's adapter; here it only
//! asks the OS whether a given number is a member of the group, pins it, reads facts through that handle and ends it. The exit code to give when ending is also
//! supplied by the caller (ADR-0275 decision 12). There are no trait objects in the public API — if a test fake is needed, the caller
//! defines a small trait and implements it on this concrete type (ADR-0266 decision 5).
//!
//! ★It uses neither logging nor locks★ — [`PinnedMember::terminate_raw`], [`PinnedMember::classify`], [`GroupRef::is_gone`]
//! are called by the caller while holding a lock, and the OS calls inside that lock are bundled into a single terminate (ADR-0262 decision 8). Do not
//! let the fact that this crate may use `tracing` (ADR-0266 decision 9) spread into this module.
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
pub(crate) use self::windows::resume_suspended_process;

/// The `Err` kind that [`GroupRef::watch_births`] and [`GroupRef::unwatch_births`] return when the owner is already gone and there is no group —
/// it is not an OS failure, so it is kept separate so the caller can tell it apart. OS failures in creating, attaching or detaching the port are not this
/// kind (they are wrapped as `Other`) — but if `start` returns this kind it cannot be told apart.
pub const GROUP_GONE: io::ErrorKind = io::ErrorKind::NotConnected;

/// The strong owner of a group — while this is alive the group exists. Dropping it on Windows closes the Job handle and the OS ends the remaining
/// members (`KILL_ON_JOB_CLOSE`) — but if a [`GroupRef`] is in a call at that moment, the handle closes when that call ends.
///
/// ★Not `Clone`★ — what the type enforces goes only as far as 「한 무리의 강한 주인은 하나」 (one strong owner per group). Who that owner is (the launch
/// path, the tree handle) is kept by the arrangement (ADR-0275 decision 11). Outward it hands out only the weak handle from [`Self::downgrade`].
///
/// Outside Windows it ties nothing together — `new`, `adopt` and `terminate` are `Ok`, `active_processes` is
/// `Unsupported`, and `downgrade` is `None`.
// ADR-0266
// ADR-0275
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

    /// Puts the process with that number into the group.
    ///
    /// ★If you put it in after launching, children it launched before being put in remain outside the group★ — the way to close this = launch it suspended, put it in, then wake it
    /// ([`crate::spawn::prepare_tree_root`], [`crate::spawn::TreeRoot`]).
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

/// The weak handle of a group — member list, pinning one member (facts, ending), join-notification port, 「사라졌나」 (is it gone).
///
/// ★It holds the group weakly★ — holding it strongly would leave the Job handle unclosed even after the owner is gone, delaying `KILL_ON_JOB_CLOSE`.
/// It is upgraded only for the duration of each call ([`Self::is_gone`] does not even upgrade) — the longest window is while [`Self::watch_births`]
/// calls `start` (so `start` must not block). If the owner is gone, the list is empty, nothing can
/// be pinned, and the port is [`GROUP_GONE`]. The pinned members and ports it hands out do not hold the group.
///
/// Outside Windows the answer is the same as a handle whose owner is gone — only the port's `Err` kind is `Unsupported`.
// ADR-0266
// ADR-0275
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

    /// The complete list of member PIDs (including the root, including members of nested groups; the order has no meaning). If the owner is gone,
    /// `Ok(빈 목록)` (empty list); if the list does not become complete within the retry limit, `Err` — so a missing member is not read as 「무리 밖」 (outside the group).
    ///
    /// ★A member that has just ended may linger briefly★ — it may leave the list later than the ending (the wait on the pinned handle returning)
    /// (CI measurement 2026-10-01). Whether it is alive is judged not by the list but by the pinned handle ([`PinnedMember::exited`]).
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

    /// Pins the process with that number **only when it is a member of this group** and reads the facts once. It does not check whether it is alive.
    ///
    /// `kill` = also opens the terminate right. `Ok(None)` = no process with that number · it is outside this group (including one that sits only in an outer
    /// Job wrapping this group) · the owner is already gone · this OS has no group. `Err` = failure to open (other than absence — access
    /// denied and the like) · failure of the membership query.
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

    /// Creates the join-notification port and passes it to `start`, and attaches the group to that port **only when `start` is `Ok`** — by the time notifications can
    /// pile up, the side that takes them out already holds the port. `start`'s `Err` is returned as is (nothing was attached).
    ///
    /// ★While `start` runs it holds the group strongly (the upgraded `Arc`)★ — even if the owner goes away meanwhile, the Job handle does not close and
    /// `KILL_ON_JOB_CLOSE` is delayed by as long as `start` blocks. So `start` only spawns the thread that takes notifications out and does not block.
    ///
    /// Members already inside are announced again as `Joined` when attaching · notifications for a member whose state changes during attaching may be missed (MS
    /// Learn). ★Call it only once per group★ — a second call is not blocked here (that is the caller's job). `Err` kinds:
    /// [`GROUP_GONE`] = the owner is already gone (`start` was not called) · `Unsupported` = this OS has no group
    /// (`start` was not called) · anything else = `start`'s error, or an OS failure in creating the port or attaching.
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

    /// Detaches the group's port connection — members born after detaching are not announced. The `Err` kinds are the same as [`Self::watch_births`].
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

    /// Is the owner gone so there is no group — it looks without upgrading the group (no side effects · it does not delay closing the Job handle meanwhile). Once
    /// true it stays true. On this OS there is no group, so it is always true.
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

/// Facts read **once** through the pinned handle — they belong to that process object, not to the number. For fields that could not be read, `ppid` and `create` =
/// 0 and strings = empty string.
///
/// - `ppid` = the parent number recorded at creation — there is no guarantee that number is still the same parent.
/// - `create` = creation time (FILETIME · 100 ns) — meaningful only for comparing the creation order of two processes.
/// - `image` = full executable path (Win32 form) · `cmdline` = the command line as is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemberFacts {
    pub ppid: u32,
    pub create: u64,
    pub image: String,
    pub cmdline: String,
}

/// The answer of [`BirthPort::next`].
///
/// - `Joined(pid)` = a member newly joined. It is only a number, so by the time it is taken out it may already have ended and been reused — it must be pinned
///   right away for the identity to stand.
/// - `Other` = any other notification (ending etc.) or a packet that is not this port's.
/// - `Timeout` = no notification arrived while waiting.
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

/// One process confirmed as a member of this group and pinned — while it is held that number is not reused, and the facts
/// remain even after it ends. It does not hold the group. Dropping it closes the handle. Only [`GroupRef::pin_member`] creates it.
///
/// Terminating is split into two methods so the caller does only **one** OS call inside a lock:
/// [`Self::terminate_raw`] = one terminate call · [`Self::classify`] = sorting that raw result.
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
    /// Has it ended now — it does not wait. `Err` = failure of the wait query itself.
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

    /// Makes **only one** terminate call, giving `exit_code`, on that pinned handle — it does not wait for the target to end. It returns the raw
    /// result only; sorting is done by [`Self::classify`]. One pinned without the terminate right (`kill = false`) is always `Err`.
    /// Failure is also returned as an error that does not use the heap.
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

    /// Sorts the raw result of [`Self::terminate_raw`] — even on failure, if it ended by itself in the meantime it is `Gone`. The handle is
    /// not closed (it closes when dropped).
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

/// The group's join-notification port — [`GroupRef::watch_births`] passes it to `start`. Several threads share it through `Arc`,
/// and when the last one is gone the port handle closes. It does not hold the group.
///
/// Notifications are only notices and delivery is not guaranteed (MS Learn) — a missed join cannot be known here.
///
/// ★Do not let go of the last one while the group is attached — detach first ([`GroupRef::unwatch_births`]), then let go★: while attached
/// the group holds the port as its own kernel reference, so the port survives even if the handle closes, and notifications keep piling up on a port nobody reads. That kernel
/// reference is not written in MS Learn and was not measured — even if wrong, detaching first loses nothing.
#[derive(Debug)]
pub struct BirthPort {
    // The inner `Arc` is shared with the share the part's `watch_births` holds until it finishes attaching — if the handle closed before that, that
    //   value could be reused and someone else's handle attached (that function's comment).
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

    /// The answer given by a handle with no owner — no members · nothing can be pinned · no port (`start` is not called). Only the kind
    /// differs per OS.
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

    // ── real processes (Windows) — each test launches only a few, one after another: do not launch them in a burst (local parallel limit `--test-threads=4` — `.claude/skill-bindings/qa.md` 「모듈별 명령」) ──

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

    /// The terminate exit code chosen by the test — it only needs to differ from terminating the whole group (1).
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

    /// Like [`open_gate`] but returns stdin without closing it — if `line` reads stdin again (`set /p`), it waits there for as long as the returned
    /// end is held.
    #[cfg(windows)]
    fn open_gate_keeping_stdin(child: &mut Child) -> ChildStdin {
        let mut stdin = child.stdin.take().expect("stdin 파이프");
        stdin.write_all(b"go\r\n").expect("문 열기");
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
            .expect("ping.exe 기동")
    }

    #[cfg(windows)]
    fn pin_ours(group: &GroupRef, pid: u32, kill: bool) -> PinnedMember {
        group
            .pin_member(pid, kill)
            .expect("붙들기")
            .unwrap_or_else(|| panic!("PID {pid} 가 우리 무리의 멤버가 아니다"))
    }

    /// Has the pinned member's parent ended — the recorded parent number cannot be pinned in our group · the pinned parent has ended · a process born later than the member
    /// holds that number. ★This assumes the parent was born in our group★ — a live parent outside the group also reads as 「끝남」 (ended).
    /// If it cannot be pinned (access denied etc.), it does not count as ended.
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
            .expect("지켜볼 핸들");
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
            .expect("붙들기")
            .is_none());
    }

    /// ② Pin the child ping of cmd in the group with the terminate right and end it — wait for the ending on the pinned handle, and the exit code leaves who
    /// ended it. It is still held after ending, so that number is not reused during the list check.
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

    /// ③ A live process of another group is not pinned (regression net for the incident in survey §4).
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

    /// ③ Nested variant — even if our group is nested under a parent Job P, a process that sits only in P is someone else's, not ours. If NULL is passed
    /// as the Job argument (「아무 Job 에나 들었나」 (is it in any Job at all)), this goes red here.
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

    /// ⑦ A port obtained through the handle — `start`'s error comes back as is (not attached), and when attaching from the received port the members already inside are
    /// announced again as `Joined`. The OS behavior of attaching and detaching itself is measured by ⑦ in the `windows` part.
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

    /// ① A parent that has ended but whose handle holds it (zombie) — it drops out of the group list and the list is complete (if a zombie remained only in the member count, the list would
    /// stay incomplete forever and cleanup would never run once). From the pinned child's view that parent has ended — if the zombie is read as alive,
    /// leftovers are missed.
    #[cfg(windows)]
    #[test]
    fn a_dead_parent_held_open_is_off_the_list_and_reads_dead() {
        let (job, group) = new_group();
        // The file that receives the output of C (ping) — P is ended only after C writes its first line. A C that has not finished starting dies
        // together when P ends (exit code 0xC000010A) and the list comes up empty (11/60 under parallel load · in runs where C's age at ending was 1 to 4 ms · measured).
        // The first line is written only by a ping that has started running its own code.
        let ready =
            std::env::temp_dir().join(format!("t40-dead-parent-{}.txt", std::process::id()));
        let _ = std::fs::remove_file(&ready);
        // P launches the ping and then lives waiting on stdin at the second `set /p` — it ends at the single kill below. Without the second `set /p`
        // `cmd /c` ends right after `start /b` and P is already dead before the probe (measured 5/5).
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
        // `parent` is still held — that handle pins P's process object.

        // Right after the wait returns, P may still be in the list (`member_pids` docs) — it waits only for it to drop out. It does not wait for
        // completeness: that must hold on every call.
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

    /// ⑤ Whether an MSYS fork child whose parent has ended stays a member of our group, whether it reads as parent-ended, whether it ends.
    /// ★It measures group inheritance only — it does not reproduce the hang★. The `bash` on `PATH` may be WSL, so the path is hard-coded.
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
        // Call `sleep` by absolute path — `usr\bin\bash.exe`, started without the Git launcher (`Git\bin\bash.exe`), has no `/usr/bin` in `PATH`,
        // so a bare `sleep` ends with 127 (command not found) (measured).
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
