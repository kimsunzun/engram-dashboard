//! Where a path spawns its child — the spawn-failure guard [`ChildGuard`] shared by the three paths (stdio · pty · codex), the group
//! adoption [`new_group_with`], and the spawning [`spawn_piped`] and stderr draining [`drain_stderr`] of the two pipe paths (stdio · codex).
//!
//! ★The group and the guard clean up the failure path after spawn and before the path stands★ — the guard is set up right after spawn, and the first failable
//!   step after spawn is group adoption. If a single `?` after adoption returns, the group ends the adopted child and the descendants spawned after it, and the guard
//!   ends the direct child (order = [`new_group_with`] doc). Both cover only this spawn **failure** path: ending after the path stands is
//!   the kill causality of the path's `shutdown()` (ADR-0001) unchanged, and on the success path the guard is released without ending anything. That placement is
//!   measured by the source test `the_guard_is_armed_and_the_child_adopted_first_after_spawn` — there is no seam to inject a failure here.
//! ★A descendant spawned before adoption is not ended on any path — this module does not close this gap★: all three spawn awake and then adopt into the group,
//!   so a grandchild that the direct child (`cmd.exe`) already spawned before adoption is outside the group (the `adopt` doc of platform `group`). On the success path
//!   closing the path does not end it either (`KILL_ON_JOB_CLOSE`), and on the failure path the guard ends only the direct child. The same holds
//!   when group adoption itself fails. The way to close it = spawn suspended (stdio · codex only — portable-pty has no such route) and is out of scope (TRD
//!   `docs/process/S21-crate-boundaries/trd-A-data-file-unification.md` §3-7 · `docs/tracking.md` T-53).
// ADR-0291

use std::io::{self, BufRead, BufReader};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use engram_dashboard_base::logging::mask_secrets;
use engram_dashboard_platform::group::GroupOwner;
use engram_dashboard_platform::spawn::hide_console_window;

use crate::output_core::OutputCore;
use crate::types::{CommandSpec, PtyError};

/// A child that [`ChildGuard`] can hold — lets a std `Child` and a portable-pty child be ended and reaped by the same rule.
///
/// ★An implementation must hold the child's process handle until it is reaped★ — [`new_group_with`] opens the child by the guard's pid, and
///   while the handle is held that pid is not handed to another process. Both close the handle when dropped.
pub(crate) trait Reap {
    /// `None` = the pid is unknown — the child cannot be put into the group.
    fn pid(&self) -> Option<u32>;
    /// Requests the direct child to end — does not wait. If the child has already ended it is `Ok`. ★On Windows `Ok` does not mean the end request
    /// took effect★ (docs of both implementations).
    fn kill_direct(&mut self) -> io::Result<()>;
    /// Reaps and returns `true` if it has ended · `false` if it is still running — does not wait. `Err` = cannot ask whether it has ended.
    fn try_reap(&mut self) -> io::Result<bool>;
}

impl Reap for Child {
    fn pid(&self) -> Option<u32> {
        Some(self.id())
    }

    /// std's Windows `kill` returns `Ok` even if `TerminateProcess` fails with access denied, as long as that child is still running (`try_wait` is
    /// not `Err`).
    fn kill_direct(&mut self) -> io::Result<()> {
        self.kill()
    }

    fn try_reap(&mut self) -> io::Result<bool> {
        Ok(self.try_wait()?.is_some())
    }
}

impl Reap for Box<dyn portable_pty::Child + Send + Sync> {
    fn pid(&self) -> Option<u32> {
        self.process_id()
    }

    /// portable-pty 0.8.1's Windows `kill` discards the result of `TerminateProcess` and always returns `Ok`.
    fn kill_direct(&mut self) -> io::Result<()> {
        self.kill()
    }

    fn try_reap(&mut self) -> io::Result<bool> {
        Ok(self.try_wait()?.is_some())
    }
}

/// Upper bound on how long the guard waits to reap the direct child it ended — past it, warn and release the handle.
///
/// ★A bound is required★ — on Windows the `Ok` of [`Reap::kill_direct`] does not mean the end request took effect, so waiting without a deadline
///   would stall the caller of spawn until a child whose kill really failed ends by itself. 2 seconds is an unmeasured margin.
const REAP_BUDGET: Duration = Duration::from_secs(2);
const REAP_POLL: Duration = Duration::from_millis(10);

/// A guard that holds the spawned child until the path takes it over — if dropped without being released, it ends the direct child and waits up to [`REAP_BUDGET`]
/// to reap it.
///
/// ★`Child` does not end the child on drop★ — wrap it with this on the line right after spawn, and release it with [`Self::into_inner`] after the last
///   step that can fail before the path stands. Ending after that is the path's job.
/// ★It ends the direct child only — the group ends the descendants★: on Windows claude · codex are wrapped by the backend with platform
///   `console_command` and run as `cmd.exe /c <CLI>`, so the real CLI is a grandchild. The step right after the guard is set up is
///   [`new_group_with`], so on a failure after adoption the group ends even the descendants spawned after adoption and this guard ends the direct child again
///   (harmless if already ended). Descendants spawned before adoption · a failed adoption · an OS with no group (outside Windows) end only the direct child
///   (module doc).
/// ★Do not find and end descendants by pid★ — `taskkill /T` (platform `process::kill_tree`) builds the tree from parent pids alone and does not look at start
///   time, so on this PC, where processes carrying a dead parent's pid are common (measured 2026-10-10 · U7 review), it kills even a stranger's tree under the
///   `cmd.exe` that got that pid again. The route of snapshotting an identity list (platform `process::subtree`) and re-reading each start time before
///   ending it also kills a stranger if the pid is handed over after the snapshot and before the read (ADR-0291 「Rejected alternatives」).
// ADR-0291
pub(crate) struct ChildGuard<C: Reap> {
    child: Option<C>,
}

impl<C: Reap> ChildGuard<C> {
    pub(crate) fn new(child: C) -> Self {
        Self { child: Some(child) }
    }

    pub(crate) fn pid(&self) -> Option<u32> {
        self.child.as_ref().and_then(Reap::pid)
    }

    pub(crate) fn child_mut(&mut self) -> &mut C {
        self.child
            .as_mut()
            .expect("the guard holds the child until it is released")
    }

    /// Releases the guard and hands the child over — after this the guard ends nothing.
    pub(crate) fn into_inner(mut self) -> C {
        self.child.take().expect("the guard is released only once")
    }
}

impl<C: Reap> Drop for ChildGuard<C> {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        if let Err(e) = child.kill_direct() {
            tracing::warn!(
                pid = ?child.pid(),
                "spawn failure cleanup: could not kill the direct child — letting it go without reaping: {e}"
            );
            return;
        }
        let deadline = Instant::now() + REAP_BUDGET;
        loop {
            match child.try_reap() {
                Ok(true) => return,
                Ok(false) if Instant::now() < deadline => std::thread::sleep(REAP_POLL),
                Ok(false) => {
                    tracing::warn!(
                        pid = ?child.pid(),
                        budget_ms = REAP_BUDGET.as_millis() as u64,
                        "spawn failure cleanup: the killed direct child did not exit within the time limit — letting it go without reaping"
                    );
                    return;
                }
                Err(e) => {
                    tracing::warn!(
                        pid = ?child.pid(),
                        "spawn failure cleanup: cannot ask whether the direct child exited — letting it go without reaping: {e}"
                    );
                    return;
                }
            }
        }
    }
}

/// Creates a new group and adopts `pid` into it. `None` = no pid to adopt (creates only an empty group).
///
/// ★Call it as the **first** failable step after spawn, right after setting up the [`ChildGuard`] that holds the spawned child★ — the later the adoption,
///   the more descendants the direct child spawns outside the group (module doc). `pid` = the guard's pid (the guard holds the handle, so the pid is not handed over
///   — [`Reap`] doc).
/// Cleanup on the failure path — the caller's locals are dropped in reverse declaration order, and the guard is declared first:
///   - If a step after this fails, the returned group owner is dropped first and the Job closes, so the OS ends the adopted child and the descendants spawned after adoption
///     (`KILL_ON_JOB_CLOSE`), then the guard ends the direct child again (harmless if already ended) and reaps it.
///   - If this fails there is no group (on a failed adoption the empty group is dropped first inside this) — the guard ends only the direct child.
// ADR-0291
pub(crate) fn new_group_with(pid: Option<u32>) -> Result<GroupOwner, PtyError> {
    let group = GroupOwner::new()?;
    if let Some(pid) = pid {
        group.adopt(pid)?;
    }
    Ok(group)
}

/// What [`spawn_piped`] hands over — the child in the group and its three pipes.
///
/// ★There is no guard from here on★ — the receiving path moves it into its own field with no failable step. Dropping this on Windows drops the group
///   owner too and ends the members, but on an OS with no group (outside Windows) the child is left running.
pub(crate) struct PipedChild {
    pub(crate) child: Child,
    pub(crate) stdin: Option<ChildStdin>,
    pub(crate) stdout: Option<ChildStdout>,
    pub(crate) stderr: Option<ChildStderr>,
    pub(crate) group: GroupOwner,
}

/// Spawns `spec` with three pipes and no window, and puts it into a new group. If adoption fails, [`ChildGuard`] ends the direct child and it returns early.
/// `what` = the head of the spawn-failure message (`"{what} spawn: …"`).
///
/// Spawns `spec` as is — the Windows shim wrapping (`cmd.exe /c …`) was already done by the backend with platform `console_command`.
// ADR-0291
pub(crate) fn spawn_piped(spec: &CommandSpec, what: &str) -> Result<PipedChild, PtyError> {
    let mut cmd = Command::new(&spec.program);
    cmd.args(&spec.args);
    cmd.current_dir(&spec.cwd);
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    // All three pipes secured — we must read stdout/stderr, or the child blocks on a full pipe buffer.
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // A headless background process — the daemon may be a windowless process, so the cmd.exe shim opens a new console.
    hide_console_window(&mut cmd);

    let child = cmd
        .spawn()
        .map_err(|e| PtyError::SpawnFailed(format!("{what} spawn: {e}")))?;
    let mut guard = ChildGuard::new(child);
    let group = new_group_with(guard.pid())?;

    let child = guard.child_mut();
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    Ok(PipedChild {
        child: guard.into_inner(),
        stdin,
        stdout,
        stderr,
        group,
    })
}

/// Spawns a thread that drains the child's stderr to the end line by line — masks each line, accumulates it into `core`'s diagnostic buffer, and
/// sends it to debug. If the thread cannot be spawned, it leaves a warn instead of swallowing the failure and returns (FIX 4). `thread_name` = the thread name · `what` = the head of that
/// warn.
///
/// ★If not drained, the child blocks on a full stderr pipe buffer and stops making progress★.
/// ★Do not mix it into the output stream (ADR-0044)★: stdout in structured mode is line-delimited JSON, so a non-JSON diagnostic line in between breaks the
///   parser.
/// ★Accumulate into the diagnostic buffer, not the output ring (ADR-0172)★: in a structured session this text is the **only evidence** of an activation failure
///   (claude's "No conversation found with session ID: …" arrives only here). Contract = `OutputCore::push_diagnostic`.
///   ★This thread is the only place that reads stderr★ — a pipe cannot be read twice, so do not make a second capture.
/// ★Masking applies to the buffer as well (FIX 4)★: it is external process output and may contain credentials, and this text may be logged again as a failure
///   reason — keeping the raw text would create a path around masking.
/// ★Level = debug (FIX 4)★: it is normal for claude to stream progress · diagnostic text to stderr — at warn it would flood the log.
// ADR-0044
// ADR-0172
pub(crate) fn drain_stderr(
    stderr: ChildStderr,
    core: &Arc<OutputCore>,
    thread_name: &str,
    what: &str,
) {
    let agent_id = core.id();
    let diag_core = core.clone();
    let spawn_result = std::thread::Builder::new()
        .name(thread_name.to_owned())
        .spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                match line {
                    Ok(l) if !l.is_empty() => {
                        let masked = mask_secrets(&l);
                        // ★Accumulating comes **before** logging★: the default log filter is warn, so the debug line below is normally discarded —
                        //   hanging the classification on that line would let the log level switch the feature on and off.
                        diag_core.push_diagnostic(&masked);
                        tracing::debug!(target: "agent_stderr", agent = %agent_id, "{}", masked)
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        });
    if let Err(e) = spawn_result {
        tracing::warn!(agent = %agent_id, "{what} stderr drain thread failed to start: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Production section only — cut before the first test module and strip everything after `//` on each line. Comments in this repo quote the names they
    /// guard verbatim, so unless stripped, a single comment line would pass for a real call.
    fn production(src: &str) -> String {
        src.split("mod tests {")
            .next()
            .expect("production section")
            .lines()
            .map(|line| line.split("//").next().unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The body of the function in `code` that starts with `signature` — from the first `{` to the matching `}`.
    fn body<'a>(code: &'a str, signature: &str) -> &'a str {
        let start = code
            .find(signature)
            .unwrap_or_else(|| panic!("`{signature}` is missing — this entry's premise is stale"));
        let open = start + code[start..].find('{').expect("opening brace of the body");
        let mut depth = 0usize;
        for (i, c) in code[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return &code[open..=open + i];
                    }
                }
                _ => {}
            }
        }
        panic!("the body of `{signature}` does not close");
    }

    fn line_at(body: &str, at: usize) -> &str {
        let from = body[..at].rfind('\n').map_or(0, |i| i + 1);
        let to = body[at..].find('\n').map_or(body.len(), |i| at + i);
        body[from..to].trim()
    }

    /// The positions where the word `word` stands in `code` — only where neither neighbor is an identifier character.
    fn word_positions(code: &str, word: &str) -> Vec<usize> {
        let ident = |c: char| c.is_alphanumeric() || c == '_';
        code.match_indices(word)
            .map(|(at, _)| at)
            .filter(|&at| {
                !code[..at].chars().next_back().is_some_and(ident)
                    && !code[at + word.len()..].chars().next().is_some_and(ident)
            })
            .collect()
    }

    /// Spellings other than `?` that leave a function early — `return` and the `else` of `let … else`. What tells them apart is right before `else`:
    /// `if … else` is `} else`, and in `let … else` the initializer cannot end with `}` (Rust grammar).
    fn early_exits(code: &str) -> Vec<usize> {
        let mut found = word_positions(code, "return");
        found.extend(
            word_positions(code, "else")
                .into_iter()
                .filter(|&at| !code[..at].trim_end().ends_with('}')),
        );
        found.sort_unstable();
        found
    }

    fn assert_no_early_exit(file: &str, code: &str, what: &str) {
        if let Some(&at) = early_exits(code).first() {
            panic!(
                "{file}: {what} has an early return that is not `?` — the source test cannot count it as a failure step: `{}`",
                line_at(code, at)
            );
        }
    }

    /// After the `?` of `spawned` — guard arming (`ChildGuard::new(`) comes before group adoption (`new_group_with(`), the first `?` is the one of group
    /// adoption, and every `?` is before guard release (`.into_inner()`). There is no early-leave spelling other than `?`.
    fn assert_adoption_is_the_first_fallible_step_after_spawn(
        file: &str,
        body: &str,
        spawned: &str,
    ) {
        let spawn_at = body.find(spawned).unwrap_or_else(|| {
            panic!("{file}: `{spawned}` is missing — this entry's premise is stale")
        });
        let spawn_step = spawn_at + body[spawn_at..].find('?').expect("the `?` of the spawn");
        let armed = body
            .find("ChildGuard::new(")
            .unwrap_or_else(|| panic!("{file}: no guard arming point"));
        let grouped = body
            .find("new_group_with(")
            .unwrap_or_else(|| panic!("{file}: no group adoption — this entry's premise is stale"));
        let released = body
            .find(".into_inner()")
            .unwrap_or_else(|| panic!("{file}: no guard release point"));
        assert!(
            spawn_step < armed,
            "{file}: the guard stands before the spawn — this entry's premise is stale"
        );
        assert!(
            armed < grouped,
            "{file}: group adoption comes before guard arming — if adoption fails, nobody kills the spawned child"
        );
        assert_no_early_exit(file, &body[spawn_step + 1..], "after spawn");
        let later: Vec<usize> = body[spawn_step + 1..]
            .match_indices('?')
            .map(|(i, _)| spawn_step + 1 + i)
            .collect();
        let first = *later.first().unwrap_or_else(|| {
            panic!("{file}: no fallible step after the spawn — this entry's premise is stale")
        });
        assert!(
            grouped < first && !body[grouped..first].contains(';'),
            "{file}: the first fallible step after the spawn is not group adoption — in that failure there is no group, so descendants launched by then are \
             killed by nobody: `{}`",
            line_at(body, first)
        );
        for &at in &later {
            assert!(
                at < released,
                "{file}: there is a fallible step after the guard is released — that failure leaves the spawned child behind: `{}`",
                line_at(body, at)
            );
        }
    }

    /// ★The guard is set up right after spawn, and the first failable step after spawn is group adoption — on all three paths★.
    ///
    /// Why it is measured in the source = that placement is a place where a failure cannot be injected (there is no seam to fail `GroupOwner::new` · `adopt` · the pty's `try_clone_reader`
    /// · `take_writer`). The real-process items below measure only what the guard and the group end, and cannot see where they
    /// stand.
    #[test]
    fn the_guard_is_armed_and_the_child_adopted_first_after_spawn() {
        let stdio = include_str!("stdio.rs");
        let pty = include_str!("pty.rs");
        let codex = include_str!("../backend/codex/transport.rs");

        let own = production(include_str!("spawn.rs"));
        assert_adoption_is_the_first_fallible_step_after_spawn(
            "transport/spawn.rs",
            body(&own, "fn spawn_piped("),
            ".spawn()",
        );
        let pty_code = production(pty);
        assert_adoption_is_the_first_fallible_step_after_spawn(
            "transport/pty.rs",
            body(&pty_code, "fn open("),
            ".spawn_command(",
        );

        for (file, src) in [
            ("transport/stdio.rs", stdio),
            ("backend/codex/transport.rs", codex),
        ] {
            let code = production(src);
            let open = body(&code, "fn open(");
            let spawned = open.find("spawn_piped(").unwrap_or_else(|| {
                panic!(
                    "{file}: `open` does not call `spawn_piped` — it could spawn outside the guard"
                )
            });
            assert!(
                !open.contains(".spawn()") && !open.contains("Command::new("),
                "{file}: `open` spawns by itself — that is outside the guard"
            );
            let spawn_step = spawned + open[spawned..].find('?').expect("the `?` of the spawn");
            assert_no_early_exit(file, &open[spawn_step + 1..], "after `spawn_piped`");
            assert!(
                !open[spawn_step + 1..].contains('?'),
                "{file}: there is a fallible step after `spawn_piped` — there is no guard there: `{}`",
                line_at(
                    open,
                    spawn_step + 1 + open[spawn_step + 1..].find('?').unwrap_or(0)
                )
            );
        }

        for (file, src) in [
            ("transport/stdio.rs", stdio),
            ("transport/pty.rs", pty),
            ("backend/codex/transport.rs", codex),
        ] {
            assert!(
                !production(src).contains("GroupOwner::new"),
                "{file}: the link creates the group directly — group adoption goes only through `new_group_with` under the guard"
            );
        }
    }

    /// The early-return finder of the item above is not blind — it catches `return` and `let … else` and does not catch `if … else`.
    #[test]
    fn the_early_exit_scan_sees_return_and_let_else_but_not_if_else() {
        assert_eq!(early_exits("let x = f();\nreturn Err(e);").len(), 1);
        assert_eq!(
            early_exits("let Some(x) = f() else {\n    panic!();\n};").len(),
            1
        );
        assert!(early_exits("let x = if a { 1 } else { 2 };").is_empty());
        assert!(early_exits("let returned = elsewhere();").is_empty());
    }

    /// A child that, although its kill is `Ok`, still answers 「still running」 in the end — the shape of a child whose kill really failed on Windows.
    struct Unending {
        polls: Arc<AtomicUsize>,
    }

    impl Reap for Unending {
        fn pid(&self) -> Option<u32> {
            None
        }

        fn kill_direct(&mut self) -> io::Result<()> {
            Ok(())
        }

        fn try_reap(&mut self) -> io::Result<bool> {
            self.polls.fetch_add(1, Ordering::Relaxed);
            Ok(false)
        }
    }

    /// ★A direct child that does not end is waited for only until the deadline and then released★ — waiting without a deadline stalls the caller of spawn.
    #[test]
    fn dropping_the_guard_gives_up_on_a_child_that_does_not_end() {
        let polls = Arc::new(AtomicUsize::new(0));
        let started = Instant::now();
        drop(ChildGuard::new(Unending {
            polls: polls.clone(),
        }));
        let took = started.elapsed();
        assert!(
            took >= REAP_BUDGET,
            "let go before the time limit — did not wait: {took:?}"
        );
        assert!(
            took < REAP_BUDGET + Duration::from_secs(2),
            "waited past the time limit: {took:?}"
        );
        assert!(
            polls.load(Ordering::Relaxed) > 1,
            "asked once and stopped — does not ask again within the time limit"
        );
    }

    // ── Real processes ──────────────────────────────────────────────────────────

    /// ★Releasing the guard ends nothing★ — ending on the success path is the path's `shutdown()` (ADR-0001).
    #[cfg(windows)]
    #[test]
    fn releasing_the_guard_leaves_the_child_running() {
        use engram_dashboard_platform::process::pid_alive;

        let child = ChildGuard::new(ping_child()).into_inner();
        assert!(
            pid_alive(child.id()),
            "the released guard killed the child — the success path kills instead of the link"
        );
        drop(ChildGuard::new(child));
    }

    /// ★Dropping the guard ends the direct child★ — at the place where group adoption failed this is the only cleanup.
    #[cfg(windows)]
    #[test]
    fn dropping_the_guard_ends_the_child() {
        assert_dropping_the_guard_ends(ping_child());
    }

    /// The same rule stands for the pty path's child (portable-pty) too.
    #[cfg(windows)]
    #[test]
    fn dropping_the_guard_ends_a_pty_child() {
        let pair = open_pty();
        let mut cmd = portable_pty::CommandBuilder::new("ping.exe");
        cmd.args(["-n", "30", "127.0.0.1"]);
        assert_dropping_the_guard_ends(pair.slave.spawn_command(cmd).expect("pty start"));
        drop(pair);
    }

    /// ★Killing an already-ended direct child is `Ok`★ — when the guard kills again a child that the group already ended after a failure post-adoption, it goes to reaping without a warn
    /// (with `Err` it does not reap).
    #[cfg(windows)]
    #[test]
    fn killing_an_already_ended_child_is_ok() {
        let mut child = ping_child();
        let _ = child.kill();
        let _ = child.wait();
        assert!(Reap::kill_direct(&mut child).is_ok());
    }

    /// ★If it fails after adoption, the direct child and the grandchild end together (the group)★ — `ping` is spawned in place of the real CLI. Only after adoption does
    /// `ping` get spawned: a gate is put in front (`spawn_gated_cmd`).
    #[cfg(windows)]
    #[test]
    fn a_failure_after_adoption_ends_the_child_and_its_grandchild() {
        use engram_dashboard_platform::testing::{open_gate, spawn_gated_cmd};
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;

        let mut seen = None;
        let failed = fail_after_adoption(
            spawn_gated_cmd("ping -n 30 127.0.0.1", CREATE_NO_WINDOW),
            |guard, group| {
                open_gate(guard.child_mut());
                seen = Some(grandchild_in(guard, group));
            },
        );
        assert_tree_ended(failed, seen);
    }

    /// The same rule stands for the pty path's child too. The gate is the same — `ping` is not spawned until `set /p` receives a line from the console,
    /// and after adoption that one line is given by a write to the master.
    #[cfg(windows)]
    #[test]
    fn a_failure_after_adoption_ends_a_pty_child_and_its_grandchild() {
        use std::io::Write;

        let pair = open_pty();
        let mut cmd = portable_pty::CommandBuilder::new("cmd.exe");
        cmd.args([
            "/d",
            "/c",
            "set",
            "/p",
            "_=",
            "&",
            "ping",
            "-n",
            "30",
            "127.0.0.1",
        ]);
        let child = pair.slave.spawn_command(cmd).expect("pty start");
        // Hold it until the decision is finished — closing the input end makes conhost tear down the session, and the grandchild may end by a path other than the group (not
        // measured).
        let mut gate = pair.master.take_writer().expect("pty write");

        let mut seen = None;
        let failed = fail_after_adoption(child, |guard, group| {
            gate.write_all(b"go\r\n").expect("open the gate");
            seen = Some(grandchild_in(guard, group));
        });
        assert_tree_ended(failed, seen);
        drop(gate);
        drop(pair);
    }

    /// Same placement as the path's `open` — set up the guard, adopt right away, call `probe`, and return as a failure. On return the locals
    /// are dropped in reverse declaration order (group → guard).
    #[cfg(windows)]
    fn fail_after_adoption<C: Reap>(
        child: C,
        probe: impl FnOnce(&mut ChildGuard<C>, &GroupOwner),
    ) -> Result<(), PtyError> {
        let mut guard = ChildGuard::new(child);
        let group = new_group_with(guard.pid())?;
        probe(&mut guard, &group);
        Err(PtyError::SpawnFailed("injected failure".into()))
    }

    /// The pids of the direct child and the grandchild `ping` under it — if the grandchild is outside the group (it was spawned before adoption), the experiment does not hold and it panics.
    #[cfg(windows)]
    fn grandchild_in<C: Reap>(guard: &ChildGuard<C>, group: &GroupOwner) -> (u32, u32) {
        let root = guard.pid().expect("direct child pid");
        let grandchild = ping_under(root);
        let members = group
            .downgrade()
            .expect("Windows has a group")
            .member_pids()
            .expect("group member list");
        assert!(
            members.contains(&grandchild),
            "grandchild ping is outside the group — it launched before adoption · the experiment does not hold: {members:?}"
        );
        (root, grandchild)
    }

    #[cfg(windows)]
    fn assert_tree_ended(failed: Result<(), PtyError>, seen: Option<(u32, u32)>) {
        use engram_dashboard_base::testing::wait_until;
        use engram_dashboard_platform::process::pid_alive;
        use std::time::Duration;

        let (root, grandchild) = seen.unwrap_or_else(|| {
            panic!("group adoption failed, so the experiment does not hold: {failed:?}")
        });
        assert!(failed.is_err(), "the injected failure did not come back");
        assert!(
            wait_until(Duration::from_secs(5), || !pid_alive(root)),
            "failed after adoption but the direct child is alive"
        );
        assert!(
            wait_until(Duration::from_secs(5), || !pid_alive(grandchild)),
            "failed after adoption but grandchild ping is alive — the group could not end a descendant launched after adoption"
        );
    }

    #[cfg(windows)]
    fn assert_dropping_the_guard_ends<C: Reap>(child: C) {
        use engram_dashboard_base::testing::wait_until;
        use engram_dashboard_platform::process::pid_alive;
        use std::time::Duration;

        let root = child.pid().expect("direct child pid");
        let started = Instant::now();
        drop(ChildGuard::new(child));
        // On a busy runner, scheduling delay may push it slightly past the deadline and the reaping can still succeed — leave margin.
        assert!(
            started.elapsed() < REAP_BUDGET + Duration::from_secs(3),
            "could not reap the direct child the guard killed within the time limit"
        );
        assert!(
            wait_until(Duration::from_secs(5), || !pid_alive(root)),
            "dropped the guard but the direct child is alive"
        );
    }

    /// `ping.exe` as the direct child — no grandchild is left, so one guard reaps everything.
    #[cfg(windows)]
    fn ping_child() -> Child {
        let mut cmd = Command::new("ping");
        cmd.args(["-n", "30", "127.0.0.1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        hide_console_window(&mut cmd);
        cmd.spawn().expect("ping start")
    }

    #[cfg(windows)]
    fn open_pty() -> portable_pty::PtyPair {
        portable_pty::native_pty_system()
            .openpty(portable_pty::PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("openpty")
    }

    /// The pid of the grandchild `ping` under `root` — if it is not seen within 10 seconds the experiment does not hold and it panics.
    #[cfg(windows)]
    fn ping_under(root: u32) -> u32 {
        use engram_dashboard_base::testing::wait_until;
        use engram_dashboard_platform::process::child_pids;
        use engram_dashboard_platform::testing::is_ping;
        use std::time::Duration;

        let mut grandchild = None;
        assert!(
            wait_until(Duration::from_secs(10), || {
                grandchild = child_pids(root).into_iter().find(|&pid| is_ping(pid));
                grandchild.is_some()
            }),
            "did not see grandchild ping within 10 seconds — the experiment does not hold"
        );
        grandchild.expect("just saw it")
    }
}
