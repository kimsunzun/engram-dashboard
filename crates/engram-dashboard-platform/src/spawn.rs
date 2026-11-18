//! OS settings when launching child processes — launching without a window · launching as a tree root and the handle that cuts that tree
//!   at once · deciding "program not found" for a failed shell · launching outside this process's Job (WMI).
// ADR-0230, ADR-0266

use std::io;
use std::path::Path;
use std::process::{Child, Command};
use std::time::Instant;

#[cfg(windows)]
mod cmd_lookup;
mod wmi;

/// Launches without a console window — Windows = `CREATE_NO_WINDOW`, other OSes = no-op (a windowless process such as the daemon launching
///   a console app otherwise opens a new console window). ★Decides the creation flags wholesale★ —
/// std's `creation_flags` replaces: flags given before are erased, and flags given again after erase this. For a tree root call
///   [`prepare_tree_root`] instead (it also launches without a window).
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

/// Prepares the child to launch as the root of a [`TreeRoot`] — Windows = no console window + suspended (wake with [`TreeRoot::start`]
///   after putting it in the group), POSIX = a new process group whose id is the child pid.
/// ★Why suspended on Windows★: a group (Job) is inherited only by processes created **after** they are inside it; launched awake, a
///   grandchild launched before being put in escapes the tree kill.
/// ★On Windows it decides the creation flags wholesale★ — as [`hide_console_window`] (do not call both).
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

/// A handle that cuts the child and all descendants at once — prepare with [`prepare_tree_root`], launch, [`Self::attach`] to the child,
///   wake with [`Self::start`]. Windows = its own group this handle owns (a Job — dropping it makes the
/// OS end the remaining members) · POSIX = the child's process group (dropping ends nothing). ★The POSIX branch has never been compiled★
///   (admission rule 3) — to kill the group with std alone and no new dependency it launches the POSIX `kill` utility.
// ADR-0275
pub struct TreeRoot {
    #[cfg(windows)]
    group: crate::group::GroupOwner,
    #[cfg(unix)]
    pgid: u32,
}

impl TreeRoot {
    /// Ties the launched child as the root of the tree. Windows `Err` = an OS failure creating the group or putting it in — then the child
    ///   stays suspended, so the caller cuts it.
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

    /// Wakes the child launched suspended — call **after** [`Self::attach`]. POSIX = nothing to wake (`Ok`). ★On Windows `Err` if not even
    ///   one suspended thread was woken★ — if [`prepare_tree_root`] was skipped (launched unsuspended),
    /// the gap before putting it in shows up as a startup failure instead of silently reviving.
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

    /// Cuts the whole tree and the child — only starts the ending and returns; check with the child's exit and [`Self::members_gone`]. The
    ///   child kill's result is discarded (also called on an already-ended child). `Err` = the tree kill
    /// (ending the Windows group · launching POSIX `kill`) failed — the child was cut then too. ★Known gap on POSIX★: called after the
    ///   child was reaped (wait), while any member remains the number is not reused, but if none remain and the
    /// number passed to a new group meanwhile it hits someone else's group (microsecond window).
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

    /// Have all descendants of the tree ended. Windows = is the group empty (true if the query fails — trusts only the child's exit) ·
    ///   POSIX = always true (std cannot count group members — trusts only the child's exit).
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

/// The "program we tried to launch is missing" decision — what it needs from the command is captured at launch, and on a non-zero exit
///   [`Self::judge`] decides.
/// ★On Windows the exit code alone cannot tell★: `cmd.exe /c <missing name>` exits differently per build (measured Windows 11 26200 =
///   **1**, the inner errorlevel 9009; 9009 is the widely known value), and 1 is
/// indistinguishable from the target's own failure. So for a failure other than 9009 it looks up the target of `cmd.exe /c <target> …` by
///   cmd's rules (working folder → `PATH`, extension = `PATHEXT`), and only when absent is it "missing"; if the
/// target cannot be singled out, only 9009 remains. ★Do not look it up before launching — do not revive★: one unresponsive remote `PATH`
///   entry held startup past the deadline (measured 21 s), and if cmd's AutoRun changes `PATH` a pre-launch
/// decision is always wrong (the post-failure decision sees the same env, so on an AutoRun-changed `PATH` it can be wrong, but only when
///   the target really failed). POSIX has nothing to decide (startup fails with `NotFound`) —
/// always [`MissingProgramVerdict::NotJudged`] outside Windows.
pub struct MissingProgramCheck {
    #[cfg(windows)]
    through_cmd: bool,
    #[cfg(windows)]
    lookup: Option<cmd_lookup::CmdLookup>,
    /// In production = one lookup list per process. Tests put in their own so parallel tests do not block each other's lookups.
    #[cfg(windows)]
    gate: &'static cmd_lookup::LookupGate,
}

/// The exit code when `cmd.exe` ends with "is not an internal or external command".
#[cfg(windows)]
const CMD_NOT_FOUND_EXIT: i32 = 9009;

impl MissingProgramCheck {
    /// The launched command as is — `env_set` is applied later than `env_remove` (a key in both stays). `cwd` = the child's working folder.
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

    /// Was the child that ended with `code` a "missing" one. The target lookup waits only until `deadline` (unknown if not finished);
    ///   success (`Some(0)`) makes no decision.
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

/// The answer of [`MissingProgramCheck::judge`]. "Missing" is [`Self::is_missing`]; the rest is material for the caller to log (this crate
///   does not log).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MissingProgramVerdict {
    /// Nothing to decide — did not go through a shell, succeeded, or the shell's target cannot be singled out as a one-word program name.
    NotJudged,
    /// The shell ended with the "missing command" exit code — "missing".
    NotFoundExitCode,
    /// The failed shell's target was looked up.
    Looked(TargetLookup),
}

impl MissingProgramVerdict {
    /// Is it "missing" — false if unknown.
    pub fn is_missing(&self) -> bool {
        matches!(
            self,
            Self::NotFoundExitCode | Self::Looked(TargetLookup::Missing)
        )
    }
}

/// The result of looking up the failed shell's target. ★The unknown branches (`Unknown`, `Busy`, `NoThread`) are not folded into "missing"★.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetLookup {
    Found,
    Missing,
    /// Unknown — could not finish looking within the deadline, or the target given as a path is remote.
    Unknown,
    /// Unknown — an earlier lookup for the same target is still running (or the lookup list is poisoned by an earlier panic), so no new
    ///   lookup was done. Usually the earlier lookup went past the deadline and is stuck in a blocked
    /// file-system query; a lookup per target runs once at a time process-wide, so stuck lookups do not pile up.
    Busy,
    /// Unknown — the lookup thread could not be launched (the message of that OS error).
    NoThread(String),
}

/// Failure of [`spawn_outside_job`].
#[derive(Debug)]
pub enum DetachedSpawnError {
    /// `Win32_Process.Create` did not launch it — `rv` = its `ReturnValue` (non-zero, e.g. 9 = path not found, 21 = invalid argument);
    ///   `u32::MAX` if the answer object or value could not be obtained.
    Refused { rv: u32 },
    /// Any other failure — COM initialization and WMI call failures are kind `Other` (message holds the HRESULT) · outside Windows =
    ///   `Unsupported`.
    Io(io::Error),
}

/// Launches `exe` outside the Job this process is in — Windows = WMI `Win32_Process.Create`. The launched process's parent is the WMI
///   provider (WmiPrvSE), not this process, so it does not inherit this process's Job (including
/// `KILL_ON_JOB_CLOSE`; measured, spike #1). It returns only whether it launched — no pid, no waiting.
/// - Command line = `exe` wrapped in quotes — no arguments, and environment variables cannot be passed this way. `exe` must be an absolute
///   path (a relative path gives `Refused { rv: 9 }`).
/// - `console` = true launches with a new console window (`CREATE_NEW_CONSOLE`); false passes no creation flags. ★False is not a no-window
///   guarantee★ — a console-subsystem exe gets a console window anyway and a windows-subsystem exe does not
///   (measured 2026-06-19); a windowless launch works only through the exe's subsystem.
/// - On the calling thread COM is initialized as a multithreaded apartment and released before returning; if the thread was already
///   initialized in another apartment mode it calls into that apartment and does not release.
/// Outside Windows = `Io` with `Unsupported`.
// ADR-0021, ADR-0271
pub fn spawn_outside_job(exe: &Path, console: bool) -> Result<(), DetachedSpawnError> {
    // ★Do not pass `CREATE_NO_WINDOW` (0x0800_0000)★ — WMI rejects it with `rv` 21 (Invalid Parameter) (measured 2026-06-17,
    //   `real_wmi_spawn_flag_matrix` in the shell's `discovery` tests); it is a direct-CreateProcess flag outside WMI Create's
    //   allowed set. `CREATE_NEW_CONSOLE` is accepted.
    const CREATE_NEW_CONSOLE: i32 = 0x0000_0010;
    let create_flags = console.then_some(CREATE_NEW_CONSOLE);
    match wmi::create(exe, create_flags)? {
        0 => Ok(()),
        rv => Err(DetachedSpawnError::Refused { rv }),
    }
}

/// The raw call of [`spawn_outside_job`] — returns `ReturnValue` as is without raising an error. `create_flags` = `None` passes no startup
///   info, `Some` passes `Win32_ProcessStartup.CreateFlags`; measuring which flags WMI rejects is the
/// diagnostic tests' job. `Err(Refused)` = a needed WMI object was empty and it stopped before launching (`rv` = `u32::MAX`).
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

    /// Only two "missing" cases — neither the unknown branches (`Unknown`, `Busy`, `NoThread`) nor "nothing to decide" fold into "missing".
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
            Looked(TargetLookup::NoThread("no thread".to_owned())),
        ] {
            assert!(!verdict.is_missing(), "{verdict:?}");
        }
    }

    /// 9009 is "missing" at once; any other failure looks up the target first; success, not-cmd and past-deadline are never "missing".
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
            "missing target + failure"
        );
        assert!(!absent.judge(Some(0), far()).is_missing(), "success");
        assert!(
            !absent.judge(Some(1), Instant::now()).is_missing(),
            "past the deadline it is unknown — not 「missing」"
        );
        let present = check("ping", r"C:\Windows\System32\CMD.EXE");
        assert!(present.judge(Some(9009), far()).is_missing());
        assert!(
            !present.judge(Some(1), far()).is_missing(),
            "a failure of an existing target is the target's own failure"
        );
        let direct = check(&missing, "other.exe");
        assert!(!direct.judge(Some(9009), far()).is_missing(), "not cmd");
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
        let mut child = cmd.spawn().expect("launch");
        let tree = TreeRoot::attach(&child).expect("put into the group");
        tree.start(&child).expect("wake");
        assert!(
            crate::group::resume_suspended_process(child.id()).is_err(),
            "a suspended thread remains even after waking"
        );

        let mut stdin = child.stdin.take().expect("stdin pipe");
        stdin.write_all(b"hello\n").expect("write");
        drop(stdin);
        let mut out = String::new();
        child
            .stdout
            .take()
            .expect("stdout pipe")
            .read_to_string(&mut out)
            .expect("read");
        assert_eq!(out.lines().collect::<Vec<_>>(), ["got:hello"]);
        let _ = child.wait();
    }

    /// ★The whole tree is inside the group★ — cmd and its child `ping` are members; launched suspended, put in, then woken, so a
    ///   grandchild launched right away is not left out. Before cutting it is not "all ended" and only after cutting does the
    ///   group empty and `ping` end — an unreadable or undercounted member count would make "all ended" stand early and the caller stop
    ///   waiting early.
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
        let mut child = cmd.spawn().expect("launch");
        let tree = match TreeRoot::attach(&child) {
            Ok(tree) => tree,
            Err(e) => {
                // Reaps the child left suspended outside the group and fails.
                let _ = child.kill();
                let _ = child.wait();
                panic!("put into the group: {e}");
            }
        };
        tree.start(&child).expect("wake");

        let members = tree.group.downgrade().expect("the group's weak handle");
        // See the ending by the pinned handle — by number, someone else's process that took over the number would read as "alive".
        let ping = wait_until("ping inside the group", || {
            let pid = members
                .member_pids()
                .ok()?
                .into_iter()
                .find(|&pid| is_ping(pid))?;
            members.pin_member(pid, false).ok()?
        });
        assert_eq!(ping.facts().ppid, child.id(), "not a ping cmd launched");
        assert!(
            members.member_pids().expect("list").contains(&child.id()),
            "cmd is outside the group"
        );
        assert!(
            !ping.exited().expect("check ping is alive"),
            "ping already ended before kill"
        );
        assert!(tree.group.active_processes().expect("member count") >= 2);
        assert!(
            !tree.members_gone(),
            "said everything ended while members are alive"
        );

        tree.kill(&mut child).expect("tree kill");
        wait_until("the group empties", || {
            matches!(tree.group.active_processes(), Ok(0)).then_some(())
        });
        assert!(
            ping.wait_exit(Duration::from_secs(10))
                .expect("wait for ping"),
            "that ping is alive even after the tree kill"
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
        let mut running = cmd.spawn().expect("launch");
        let result = crate::group::resume_suspended_process(running.id());
        let _ = running.kill();
        let _ = running.wait();
        assert!(
            result.is_err(),
            "said it woke a process that was not suspended"
        );
    }
}
