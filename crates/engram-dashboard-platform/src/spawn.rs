//! OS settings when launching child processes — launching without a window · launching as a tree root and the handle that cuts that tree at once ·
//! deciding 「프로그램 없음」 (program not found) for a failed shell · launching outside this process's Job (WMI).
// ADR-0230
// ADR-0266

use std::io;
use std::path::Path;
use std::process::{Child, Command};
use std::time::Instant;

#[cfg(windows)]
mod cmd_lookup;
mod wmi;

/// Launches without a console window — Windows = `CREATE_NO_WINDOW`, other OSes = no-op. If a windowless process (the daemon) simply launches a console app,
/// Windows opens a new console window.
///
/// ★It decides the creation flags wholesale★ — std's `creation_flags` replaces rather than adds. Creation flags given before this are erased by it,
/// and if creation flags are given again after this, this is erased. When launching as a tree root, call
/// [`prepare_tree_root`] instead of this (it also does the launch without a window).
pub fn hide_console_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows::Win32::System::Threading::CREATE_NO_WINDOW;
        cmd.creation_flags(CREATE_NO_WINDOW.0);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

/// Prepares the child to launch as the root of a [`TreeRoot`] — Windows = no console window + suspended (wake it with
/// [`TreeRoot::start`] after putting it in the group), POSIX = a new process group whose id is the child pid.
///
/// ★Why launch suspended on Windows★ — a group (Job) is inherited only by processes created **after** they are inside it. If it is
///   launched awake, a grandchild the child launches before being put in escapes the tree kill.
/// ★On Windows it decides the creation flags wholesale★ — same as [`hide_console_window`] (do not call it together with that).
pub fn prepare_tree_root(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows::Win32::System::Threading::{CREATE_NO_WINDOW, CREATE_SUSPENDED};
        cmd.creation_flags(CREATE_NO_WINDOW.0 | CREATE_SUSPENDED.0);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
}

/// A handle that cuts the child and all its descendants at once — prepare with [`prepare_tree_root`], launch, [`Self::attach`] it to the child,
/// and wake it with [`Self::start`]. Windows = its own group that this handle owns (a Job — dropping it makes the OS end the remaining members) ·
/// POSIX = the child's process group (dropping it ends nothing).
///
/// ★The POSIX branch has never been compiled★ (this crate's admission rule 3) — to do the group kill with std alone and no new dependency, it launches the POSIX
///   `kill` utility.
// ADR-0275
pub struct TreeRoot {
    #[cfg(windows)]
    group: crate::group::GroupOwner,
    #[cfg(unix)]
    pgid: u32,
}

impl TreeRoot {
    /// Ties the launched child as the root of the tree. Windows `Err` = an OS failure in creating the group or putting it in — then the child remains
    /// suspended, so the caller cuts it.
    pub fn attach(child: &Child) -> io::Result<Self> {
        #[cfg(windows)]
        {
            let group = crate::group::GroupOwner::new()?;
            group.adopt(child.id())?;
            Ok(Self { group })
        }
        #[cfg(unix)]
        {
            Ok(Self { pgid: child.id() })
        }
    }

    /// Wakes the child launched suspended — call it **after** [`Self::attach`]. POSIX = nothing to wake (`Ok`).
    ///
    /// ★On Windows it is `Err` if not even one suspended thread was woken★ — if [`prepare_tree_root`] was skipped and the child was launched
    ///   unsuspended, the gap before putting it in does not silently revive but shows up as a startup failure.
    pub fn start(&self, child: &Child) -> io::Result<()> {
        #[cfg(windows)]
        {
            crate::group::resume_suspended_process(child.id())
        }
        #[cfg(unix)]
        {
            let _ = child;
            Ok(())
        }
    }

    /// Cuts the whole tree and cuts the child too — it only starts the ending and returns. Whether everything ended is checked by the child's exit and
    /// [`Self::members_gone`]. The result of the child kill is discarded — it is also called on an already-ended child.
    ///
    /// `Err` = the tree kill (ending the Windows group · launching POSIX `kill`) failed — the child was cut then too.
    ///
    /// ★A known gap on POSIX★: if called after the child has already been reaped (wait) — while any member remains that number is not reused,
    ///   but if none remain and the number passed to a new group in the meantime, it hits someone else's group (window = microseconds).
    pub fn kill(&self, child: &mut Child) -> io::Result<()> {
        #[cfg(windows)]
        let tree = self.group.terminate(1);
        #[cfg(unix)]
        let tree = {
            use std::process::Stdio;
            // A negative pid = that process group. Without `--`, `-<id>` is read as a signal name.
            let group = format!("-{}", self.pgid);
            Command::new("kill")
                .args(["-s", "KILL", "--", &group])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(drop)
        };
        let _ = child.kill();
        tree
    }

    /// Have all descendants of the tree ended. Windows = is the group empty (true if the query fails — it trusts only the child's exit) · POSIX = always
    /// true (std has no means to count group members — it trusts only the child's exit).
    pub fn members_gone(&self) -> bool {
        #[cfg(windows)]
        {
            self.group.active_processes().map_or(true, |n| n == 0)
        }
        #[cfg(unix)]
        {
            true
        }
    }
}

/// The 「띄우려던 프로그램이 없다」 (the program we tried to launch is missing) decision — what it needs from the command is captured at launch, and when a non-zero exit is received it is decided by
/// [`Self::judge`].
///
/// ★On Windows the exit code alone cannot tell apart★ — the exit code of `cmd.exe /c <없는 이름>` differs per build
///   (measured 2026-09-27, Windows 11 26200 = **1**, the errorlevel inside it is 9009 · the widely known value is 9009). 1 is
///   indistinguishable from the target program's own failure. So for a failure that is not 9009, it looks up the target of `cmd.exe /c <대상> …` by the same
///   rules as cmd (working folder → `PATH`, extension = `PATHEXT`), and only when it is absent is it 「없다」 (missing). If the shape is one where the target cannot be singled out,
///   only 9009 remains.
/// ★Do not look it up before launching — do not revive that★: one unresponsive remote `PATH` entry held startup past the deadline
///   (measured 21 s), and if cmd's AutoRun changes `PATH`, a decision before launching is always wrong. The decision after the failure sees the same env too, so
///   on a PC where AutoRun changed `PATH` it can be wrong — but only when the target really failed.
/// POSIX has nothing to decide, because startup fails with `NotFound` — outside Windows it is always
/// [`MissingProgramVerdict::NotJudged`].
pub struct MissingProgramCheck {
    #[cfg(windows)]
    through_cmd: bool,
    #[cfg(windows)]
    lookup: Option<cmd_lookup::CmdLookup>,
    /// In production = one lookup list per process. Tests put in their own — so that tests running in parallel do not block each other's lookups.
    #[cfg(windows)]
    gate: &'static cmd_lookup::LookupGate,
}

/// The exit code when `cmd.exe` ends with 「내부 또는 외부 명령 … 이 아닙니다」 (is not an internal or external command).
#[cfg(windows)]
const CMD_NOT_FOUND_EXIT: i32 = 9009;

impl MissingProgramCheck {
    /// The launched command as is — `env_set` is treated as applied later than `env_remove` (if the same key is in both lists
    /// it stays). `cwd` = the child's working folder.
    pub fn new(
        program: &str,
        args: &[String],
        env_set: &[(String, String)],
        env_remove: &[String],
        cwd: &Path,
    ) -> Self {
        #[cfg(windows)]
        {
            Self::with_gate(
                program,
                args,
                env_set,
                env_remove,
                cwd,
                &cmd_lookup::LOOKUPS_IN_FLIGHT,
            )
        }
        #[cfg(not(windows))]
        {
            let _ = (program, args, env_set, env_remove, cwd);
            Self {}
        }
    }

    #[cfg(windows)]
    fn with_gate(
        program: &str,
        args: &[String],
        env_set: &[(String, String)],
        env_remove: &[String],
        cwd: &Path,
        gate: &'static cmd_lookup::LookupGate,
    ) -> Self {
        Self {
            through_cmd: cmd_lookup::is_cmd_shell(program),
            lookup: cmd_lookup::CmdLookup::from_command(program, args, env_set, env_remove, cwd),
            gate,
        }
    }

    /// Was the child that ended with `code` a 「없다」 (missing) one. Looking up the target waits only until `deadline` — if it cannot finish within the deadline, unknown.
    /// If successful (`Some(0)`), no decision is made.
    pub fn judge(&self, code: Option<i32>, deadline: Instant) -> MissingProgramVerdict {
        #[cfg(windows)]
        {
            if !self.through_cmd || code == Some(0) {
                return MissingProgramVerdict::NotJudged;
            }
            if code == Some(CMD_NOT_FOUND_EXIT) {
                return MissingProgramVerdict::NotFoundExitCode;
            }
            match &self.lookup {
                Some(lookup) => {
                    MissingProgramVerdict::Looked(lookup.run_within(deadline, self.gate))
                }
                None => MissingProgramVerdict::NotJudged,
            }
        }
        #[cfg(not(windows))]
        {
            let _ = (code, deadline);
            MissingProgramVerdict::NotJudged
        }
    }
}

/// The answer of [`MissingProgramCheck::judge`]. Whether it is 「없다」 (missing) is [`Self::is_missing`], and the rest is material for the caller to log
/// (this crate does not log).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MissingProgramVerdict {
    /// Nothing to decide — it did not go through a shell · it succeeded · the shell's target cannot be singled out as a one-word program name.
    NotJudged,
    /// The shell ended with the 「없는 명령」 (missing command) exit code — 「없다」 (missing).
    NotFoundExitCode,
    /// The failed shell's target was looked up.
    Looked(TargetLookup),
}

impl MissingProgramVerdict {
    /// Is it 「없다」 (missing) — if unknown, false.
    pub fn is_missing(&self) -> bool {
        matches!(
            self,
            Self::NotFoundExitCode | Self::Looked(TargetLookup::Missing)
        )
    }
}

/// The result of looking up the failed shell's target. ★The unknown branches (`Unknown` · `Busy` · `NoThread`) are not folded into 「없다」 (missing)★.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetLookup {
    Found,
    Missing,
    /// Unknown — it could not finish looking within the deadline, or the target given as a path is a remote location.
    Unknown,
    /// Unknown — an earlier lookup for the same target is still running (or the lookup list is poisoned by an earlier panic), so no new lookup was done.
    /// Usually the earlier decision's lookup went past the deadline and is stuck in a blocked file-system query — a lookup for the same target runs once at a time across the whole
    /// process, so stuck lookups do not pile up with each decision.
    Busy,
    /// Unknown — the lookup thread could not be launched (the message of that OS error).
    NoThread(String),
}

/// Failure of [`spawn_outside_job`].
#[derive(Debug)]
pub enum DetachedSpawnError {
    /// `Win32_Process.Create` did not launch it — `rv` = its `ReturnValue` (a non-zero value · e.g. 9 = path not found ·
    /// 21 = invalid argument). If the answer object or that value could not be obtained, it is `u32::MAX`.
    Refused { rv: u32 },
    /// Any other failure — failures of COM initialization and the WMI call are kind `Other` (the message holds that HRESULT) · outside Windows =
    /// `Unsupported` (this means is absent).
    Io(io::Error),
}

/// Launches `exe` outside the Job this process is in — Windows = WMI `Win32_Process.Create`. The launched process's parent is not this
/// process but the WMI provider (WmiPrvSE), so it does not inherit this process's Job (including `KILL_ON_JOB_CLOSE`) (measured —
/// spike #1). It returns only whether it launched — no pid, and no waiting for the launched process.
///
/// - Command line = just `exe` wrapped in quotes — no arguments are passed. Environment variables cannot be passed by this means.
/// - `exe` must be an absolute path — a relative path gives `Refused { rv: 9 }` (Path not found).
/// - `console` = if true, launch with a new console window (`CREATE_NEW_CONSOLE`) · if false, no creation flags are passed.
///   ★False is not a guarantee that no window appears★ — a console-subsystem exe gets a console window even when launched this way, and a windows
///   subsystem exe does not (measured 2026-06-19). Launching without a window works only through the exe's subsystem.
/// - On the calling thread, COM is initialized as a multithreaded apartment and released before returning. If the thread was already
///   initialized in another apartment mode, it calls into that apartment and does not release.
///
/// Outside Windows = `Io` with `Unsupported`.
// ADR-0021
// ADR-0271
pub fn spawn_outside_job(exe: &Path, console: bool) -> Result<(), DetachedSpawnError> {
    // ★Do not pass `CREATE_NO_WINDOW` (0x0800_0000)★ — WMI rejects it with `rv` 21 (Invalid Parameter) (measured
    //   2026-06-17 · `real_wmi_spawn_flag_matrix` in the shell's `discovery` tests). It is a flag for direct CreateProcess calls, so it is outside WMI Create's
    //   allowed set. `CREATE_NEW_CONSOLE` is accepted.
    const CREATE_NEW_CONSOLE: i32 = 0x0000_0010;
    let create_flags = console.then_some(CREATE_NEW_CONSOLE);
    match wmi::create(exe, create_flags)? {
        0 => Ok(()),
        rv => Err(DetachedSpawnError::Refused { rv }),
    }
}

/// The raw call of [`spawn_outside_job`] — it returns `ReturnValue` as is without raising it as an error. `create_flags` =
/// `None` passes no startup info, `Some` passes it as `Win32_ProcessStartup.CreateFlags`. Measuring which creation
/// flags WMI rejects is the job of the diagnostic tests. `Err(Refused)` = a needed WMI object was empty and it stopped before launching
/// (`rv` = `u32::MAX`).
#[cfg(any(test, feature = "test-support"))]
pub fn wmi_create_raw(exe: &Path, create_flags: Option<i32>) -> Result<u32, DetachedSpawnError> {
    wmi::create(exe, create_flags)
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use std::io::{Read, Write};
    #[cfg(windows)]
    use std::process::Stdio;
    #[cfg(windows)]
    use std::time::Duration;

    use super::*;

    #[cfg(windows)]
    fn far() -> Instant {
        Instant::now() + Duration::from_secs(20)
    }

    /// There are only two 「없다」 (missing) cases — neither the unknown branches (`Unknown` · `Busy` · `NoThread`) nor "nothing to decide" fold into 「없다」 (missing).
    #[test]
    fn only_a_not_found_exit_or_a_missing_target_is_missing() {
        use MissingProgramVerdict::{Looked, NotFoundExitCode, NotJudged};
        assert!(NotFoundExitCode.is_missing());
        assert!(Looked(TargetLookup::Missing).is_missing());
        for verdict in [
            NotJudged,
            Looked(TargetLookup::Found),
            Looked(TargetLookup::Unknown),
            Looked(TargetLookup::Busy),
            Looked(TargetLookup::NoThread("스레드 없음".to_owned())),
        ] {
            assert!(!verdict.is_missing(), "{verdict:?}");
        }
    }

    /// 9009 is 「없다」 (missing) right away; any other failure looks up the target first; success, not-cmd and past-deadline are never 「없다」 (missing).
    #[cfg(windows)]
    #[test]
    fn the_judgment_reads_9009_and_looks_only_after_a_failure() {
        let cwd = std::env::temp_dir();
        let missing = format!("engram-no-such-program-{}", uuid::Uuid::new_v4().simple());
        let check = |target: &str, program: &str| {
            let args = ["/c", target, "--flag"].map(str::to_owned);
            MissingProgramCheck::with_gate(
                program,
                &args,
                &[],
                &[],
                &cwd,
                cmd_lookup::LookupGate::own(),
            )
        };
        let absent = check(&missing, "cmd.exe");
        assert!(absent.judge(Some(9009), far()).is_missing());
        assert!(
            absent.judge(Some(1), far()).is_missing(),
            "없는 대상 + 실패"
        );
        assert!(!absent.judge(Some(0), far()).is_missing(), "성공");
        assert!(
            !absent.judge(Some(1), Instant::now()).is_missing(),
            "마감이 지나면 모른다 — 「없다」가 아니다"
        );
        let present = check("ping", r"C:\Windows\System32\CMD.EXE");
        assert!(present.judge(Some(9009), far()).is_missing());
        assert!(
            !present.judge(Some(1), far()).is_missing(),
            "있는 대상의 실패는 대상 자신의 실패다"
        );
        let direct = check(&missing, "other.exe");
        assert!(
            !direct.judge(Some(9009), far()).is_missing(),
            "cmd 가 아니다"
        );
        assert!(!direct.judge(Some(1), far()).is_missing());
    }

    /// A child launched suspended comes through awake — it talks, and no suspended thread is left (waking again = failure).
    #[cfg(windows)]
    #[test]
    fn the_suspended_spawn_hands_over_a_running_child() {
        let mut cmd = Command::new("cmd.exe");
        cmd.args(["/v:on", "/c", "set /p L=& echo got:!L!"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        prepare_tree_root(&mut cmd);
        let mut child = cmd.spawn().expect("기동");
        let tree = TreeRoot::attach(&child).expect("무리에 넣기");
        tree.start(&child).expect("깨우기");
        assert!(
            crate::group::resume_suspended_process(child.id()).is_err(),
            "깨운 뒤에도 멈춘 스레드가 남았다"
        );

        let mut stdin = child.stdin.take().expect("stdin 파이프");
        stdin.write_all(b"hello\n").expect("쓰기");
        drop(stdin);
        let mut out = String::new();
        child
            .stdout
            .take()
            .expect("stdout 파이프")
            .read_to_string(&mut out)
            .expect("읽기");
        assert_eq!(out.lines().collect::<Vec<_>>(), ["got:hello"]);
        let _ = child.wait();
    }

    /// ★The whole tree is inside the group★ — cmd and its child `ping` are members of the group. It is launched suspended, put in, and then woken, so a grandchild launched right away
    ///   is not left out either. Before cutting it is not 「다 끝났다」 (all ended), and only after cutting does the group empty and that `ping` end —
    ///   if the member count cannot be read or is undercounted, 「다 끝났다」 (all ended) stands early and the caller stops waiting early.
    #[cfg(windows)]
    #[test]
    fn the_tree_holds_every_member_until_it_is_killed() {
        use crate::testing::{is_ping, wait_until};

        let mut cmd = Command::new("cmd.exe");
        cmd.args(["/v:on", "/c", "ping -n 120 127.0.0.1 > NUL"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        prepare_tree_root(&mut cmd);
        let mut child = cmd.spawn().expect("기동");
        let tree = match TreeRoot::attach(&child) {
            Ok(tree) => tree,
            Err(e) => {
                // Reaps the child left suspended outside the group and fails.
                let _ = child.kill();
                let _ = child.wait();
                panic!("무리에 넣기: {e}");
            }
        };
        tree.start(&child).expect("깨우기");

        let members = tree.group.downgrade().expect("무리의 약한 손잡이");
        // See the ending by the pinned handle — by number, someone else's process that took over the number after the end would read as 「산다」 (alive).
        let ping = wait_until("무리 안의 ping", || {
            let pid = members
                .member_pids()
                .ok()?
                .into_iter()
                .find(|&pid| is_ping(pid))?;
            members.pin_member(pid, false).ok()?
        });
        assert_eq!(ping.facts().ppid, child.id(), "cmd 가 띄운 ping 이 아니다");
        assert!(
            members.member_pids().expect("명단").contains(&child.id()),
            "cmd 가 무리 밖이다"
        );
        assert!(
            !ping.exited().expect("ping 생존 확인"),
            "kill 전에 ping 이 이미 끝났다"
        );
        assert!(tree.group.active_processes().expect("멤버 수") >= 2);
        assert!(!tree.members_gone(), "멤버가 사는데 다 끝났다고 했다");

        tree.kill(&mut child).expect("트리 kill");
        wait_until("무리가 빈다", || {
            matches!(tree.group.active_processes(), Ok(0)).then_some(())
        });
        assert!(
            ping.wait_exit(Duration::from_secs(10)).expect("ping 대기"),
            "트리 kill 뒤에도 그 ping 이 산다"
        );
        assert!(tree.members_gone());
        let _ = child.wait();
    }

    /// ★Asking to wake a process that was never suspended fails★ — if launch-suspended is lost, startup shows up as a failure.
    /// Launch `ping` directly without a shell — through a shell only the shell is cut and the grandchild remains (this test has no group).
    #[cfg(windows)]
    #[test]
    fn resuming_a_process_that_was_never_suspended_fails() {
        let mut cmd = Command::new("ping.exe");
        cmd.args(["-n", "30", "127.0.0.1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        hide_console_window(&mut cmd);
        let mut running = cmd.spawn().expect("기동");
        let result = crate::group::resume_suspended_process(running.id());
        let _ = running.kill();
        let _ = running.wait();
        assert!(result.is_err(), "멈추지 않은 프로세스를 깨웠다고 했다");
    }
}
