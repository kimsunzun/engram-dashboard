//! PtyTransport — the AgentTransport implementation shared by console backends (claude/codex/gemini). Zero tauri imports, zero unsafe.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use engram_dashboard_base::panic::{contain, Caught};
use engram_dashboard_base::sync;
use engram_dashboard_platform::group::GroupOwner;
use engram_dashboard_platform::pty::CLOSE_NEEDS_DRAIN;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

use crate::output_core::OutputCore;
use crate::transport::input_queue::{self, InputQueue};
use crate::transport::spawn::{new_group_with, ChildGuard};
use crate::transport::AgentTransport;
use crate::types::{
    CommandSpec, ControlCaps, InputCaps, InputEvent, OutputCaps, OutputEvent, PtyError,
    TerminalReason, TransportCaps,
};

/// `child` is shared by the pump (try_wait) and shutdown (kill+wait) as Arc<Mutex>; the shutdown flag is an Arc too — shutdown sets
///   (Release), the pump's termination part reads (Acquire).
pub struct PtyTransport {
    /// `master` can be taken by both the watcher (natural-exit detection) and shutdown (kill), so it is Arc-shared; both `take()`, so it
    ///   is idempotent — the first closes the ConPTY, the later sees None.
    master: Arc<Mutex<Option<Box<dyn MasterPty + Send>>>>,
    /// ★The writer thread **owns** it — taken in `start()` and moved; `None` afterwards★.
    ///
    /// ★Owned, not `Arc<Mutex<…>>`★: `portable_pty`'s writer **can be taken only once** (trait doc; unix `bail!`, Windows ConPTY
    ///   `Option::take()`) and is `Send` but not `Sync`, so it belongs to **one thread**, the writer. This differs from the
    ///   stdio side's `Arc<Mutex<Option<ChildStdin>>>` (there `shutdown()`'s `try_lock` cleanup requires sharing).
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    input: Arc<InputQueue>,
    child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
    shutdown: Arc<AtomicBool>,
    /// Set (Release) by `shutdown()` once the master is destroyed; the pump loads it (Acquire) to know when it may stop draining — only
    ///   where [`CLOSE_NEEDS_DRAIN`] holds (rule = [`run_pump`]).
    master_closed: Arc<AtomicBool>,
    reader: Mutex<Option<Box<dyn Read + Send>>>,
    /// ★Has this path ever **accepted** input — the latch of 「the child has not yet printed any user · model content」★.
    ///
    /// ★Only tests read it today — do not delete★: the fact can be known only by the path (the only place seeing both input and output
    ///   sides) and cannot be rebuilt; its only production consumer, the pre-input observation window, was withdrawn by ADR-0217.
    /// ★Raise it the moment the queue **accepts**, not when the OS write finishes★ (the write time is unknown; raising late would count
    ///   bytes read in between as 「before input」).
    /// ★**Do not raise** it for rejected input★ — bytes sent back (queue closed or over the cap; the cap is reachable —
    ///   `tests/transport_smoke.rs`) never reach the PTY, so the premise stays.
    /// ★Write `Release`★, read with `Acquire`, so 「input was accepted」 orders against the observer instead of leaning on an accidental barrier.
    input_seen: Arc<AtomicBool>,
    /// The owner of the process group this path launched. ★Keep it the last field★ — fields drop in declaration order, which fixes the
    ///   moment the group closes (the OS ends the remaining members) when the path is dropped. Moving it earlier is
    /// unmeasured and no test guards this arrangement.
    group: GroupOwner,
}

impl PtyTransport {
    /// **The pump is not launched yet** (in start). Also returns child_pid (for claude session tracking by the caller).
    pub fn open(
        spec: &CommandSpec,
        cols: u16,
        rows: u16,
    ) -> Result<(PtyTransport, Option<u32>), PtyError> {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| PtyError::SpawnFailed(format!("openpty: {e}")))?;

        let cmd = build_pty_command(spec);
        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| PtyError::SpawnFailed(format!("spawn: {e}")))?;
        let guard = ChildGuard::new(child);
        // ★The first step that can fail after launch = putting into the group; declare the guard before the group★ — a later failure drops
        //   locals in reverse order (group → guard). Reason = the `new_group_with` doc.
        let child_pid = guard.pid();
        let group = new_group_with(child_pid)?;

        // The slave is unneeded after spawn — dropping it prevents an FD leak (and is needed for a normal ConPTY EOF).
        drop(pair.slave);

        // ★Secure the reader/writer first, before loading the master★.
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| PtyError::SpawnFailed(format!("clone_reader: {e}")))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| PtyError::SpawnFailed(format!("take_writer: {e}")))?;

        let transport = PtyTransport {
            master: Arc::new(Mutex::new(Some(pair.master))),
            writer: Mutex::new(Some(writer)),
            input: Arc::new(InputQueue::new()),
            child: Arc::new(Mutex::new(guard.into_inner())),
            shutdown: Arc::new(AtomicBool::new(false)),
            master_closed: Arc::new(AtomicBool::new(false)),
            reader: Mutex::new(Some(reader)),
            input_seen: Arc::new(AtomicBool::new(false)),
            group,
        };

        Ok((transport, child_pid))
    }
}

/// Split out of open() so the terminal-declaration env default injection (load-bearing) is unit-testable without a real spawn.
fn build_pty_command(spec: &CommandSpec) -> CommandBuilder {
    let mut cmd = CommandBuilder::new(&spec.program);
    for a in &spec.args {
        cmd.arg(a);
    }
    cmd.cwd(&spec.cwd);
    // ADR-0184: the PTY transport declares a terminal identity to the child (the four rejected alternatives = that ADR).
    // The daemon is launched through WMI (WmiPrvSE.exe parent), so the inherited environment is the **service environment** with no TERM ·
    //   COLORTERM. portable-pty passes the whole inherited environment (+ HKLM/HKCU `Environment` hives on Windows)
    //   but **does not synthesize TERM** (only unix's SHELL). Without it the codex TUI loses the header box's lower border and the prompt
    //   background block (measured 2026-09-08: just these two variables match Windows Terminal). This layer hands out the
    //   PTY, so it asserts "what terminal we are" for every backend.
    // ★Inherited values are overridden★ — the child draws onto our xterm.js widget, not whatever launched the daemon; only the profile
    //   (spec.env) wins over the default.
    // Follows ADR-0049's env default discipline — ★explicit-skip★: a key the profile already gives is not injected (deterministic, not
    //   merge-order last-wins). ★Case-insensitive★: Windows env names are; on unix a lowercase `term` would not override `TERM`
    //   and **both** would go out in the child block — `eq_ignore_ascii_case` blocks that.
    for (key, value) in [("TERM", "xterm-256color"), ("COLORTERM", "truecolor")] {
        if !spec.env.iter().any(|(k, _)| k.eq_ignore_ascii_case(key)) {
            cmd.env(key, value);
        }
    }
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    cmd
}

/// Closes the input queue **whichever way the pump ends** and reaps the writer thread.
///
/// ★A `Drop` on purpose★ — normal EOF or a pump-body panic (absorbed by `contain`) both pass here, as the codex path's `ReaderExit`; ★if
///   missed, **a writer thread remains forever for every naturally exited agent**★ (`shutdown()` is not called from the
///   reaper path, so nothing else wakes it, and it holds the queue and the PTY writer handle).
struct WriterStop(Arc<InputQueue>);

impl Drop for WriterStop {
    fn drop(&mut self) {
        self.0.close("the PTY stream ended — nowhere left to send");
    }
}

/// B-2. Split out of the pump closure so the (panic→Failed) mapping is unit-testable without a real PTY child.
fn resolve_pump_reason(result: Result<TerminalReason, Caught>) -> TerminalReason {
    match result {
        Ok(reason) => reason,
        Err(caught) => TerminalReason::Error(format!("pump panicked: {caught}")),
    }
}

/// The pump body: reads until the stream ends or the stop rule fires, then names the reason.
///
/// - Bytes read before `shutdown` is set go to `emit`; after it, discarded. EOF or a read error ends the loop.
/// - Once `shutdown` is set: `close_needs_drain` false → the first read that returns ends the loop; true → keep reading and discarding
///   until the first read that returns after `master_closed` is set — `ClosePseudoConsole` waits until
///   its output has been read, so an earlier stop can block the close (and the kill) forever (T-79).
/// - The reason is `Killed` when `shutdown` is set at the end, otherwise `Exited` with `exit_code()` (called either way).
// ADR-0001
fn run_pump(
    mut reader: impl Read,
    shutdown: &AtomicBool,
    master_closed: &AtomicBool,
    close_needs_drain: bool,
    mut emit: impl FnMut(&[u8]),
    exit_code: impl FnOnce() -> Option<i32>,
) -> TerminalReason {
    let mut buf = [0u8; 4096];
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if shutdown.load(Ordering::Relaxed) {
            if !close_needs_drain || master_closed.load(Ordering::Acquire) {
                break;
            }
            continue;
        }
        emit(&buf[..n]);
    }

    let code = exit_code();
    if shutdown.load(Ordering::Acquire) {
        TerminalReason::Killed
    } else {
        TerminalReason::Exited { code }
    }
}

impl AgentTransport for PtyTransport {
    fn start(&self, core: Arc<OutputCore>) {
        let reader = match self.reader.lock().expect("reader poisoned").take() {
            Some(r) => r,
            None => return,
        };

        // ── Input writer thread ──
        // ★Nobody joins it★ — `core` holds only one pump handle, and waiting inside `shutdown()` violates ADR-0001's 2-verb contract. It
        //   ends by itself when the queue closes (`WriterStop`'s `Drop` and `shutdown()`); a hanging `write_all` is released as an
        //   error by kill+master drop. ★So it **cannot delay the kill causality**★.
        // ★Launched in `start()`, not `open()`★ — like the codex writer, separating 「what made the path」 from 「what started it」; input
        //   before `start()` waits in the queue and flows out here (ADR-0190 discipline 1).
        if let Some(mut writer) = self.writer.lock().expect("writer poisoned").take() {
            let queue = self.input.clone();
            // ★The id must be passed in to attribute the writer's warnings★ — `core` cannot be held inside (only queue and writer move to
            //   the thread); same `agent = %core.id()` field as the spawn failure branch below.
            let writer_agent = core.id();
            let spawn_result = std::thread::Builder::new()
                .name("engram-pty-writer".into())
                .spawn(move || {
                    input_queue::drain(&queue, "PTY", writer_agent, |bytes| {
                        writer.write_all(bytes)?;
                        writer.flush()
                    });
                });
            if let Err(e) = spawn_result {
                // ★Not silent★: with no writer `send_input` would keep returning `Ok` while bytes never go out — a silent loss. Close the
                //   queue and **reject honestly from then**.
                let reason = format!("failed to start the PTY input writer thread: {e}");
                tracing::warn!(agent = %core.id(), "{reason}");
                self.input.close(&reason);
            }
        }

        let (done_tx, done_rx) = mpsc::channel();
        let pump_core = core.clone();
        let child = self.child.clone();
        let shutdown = self.shutdown.clone();
        let master_closed = self.master_closed.clone();
        let writer_stop = WriterStop(self.input.clone());

        // ── Natural-exit detection watcher (console only — kept inside PtyTransport) ──
        // Problem: on Windows ConPTY, while the master is alive a child that exits by itself (cmd /c exit) gives the reader no EOF, so the
        //   pump's blocking read never wakes → core.finish is not called → no reaper signal. (Kill path: shutdown drops the master → EOF.)
        // Fix: poll for child termination and **drop the master**, taking the existing EOF→pump break→finish→reaper path.
        //
        // ★The shutdown flag is not touched★ — set, the pump reports Killed; a natural exit must be exactly Exited{code} for status · logs
        //   (ADR-0083: reaper disposition is KeepAsIs only for shutdown, KeepDisableAutoRestore otherwise; the exit code is still
        //   needed for diagnostics). The watcher only drops the master; the pump produces the reason from try_wait's exit code.
        //
        // ★Deadlock/double-wait safety★: WinChild::try_wait/wait/kill try_clone the internal proc handle and release the outer Mutex at
        //   once, so the watcher holds our child Mutex **only briefly** (one try_wait) and sleeps — no deadlock against shutdown's
        //   child.lock()+kill+wait. Windows has no zombie reaping and holding the handle preserves the exit code, so repeated retrieval by
        //   watcher and pump is harmless.
        let watcher_child = self.child.clone();
        let watcher_master = self.master.clone();
        let watcher_shutdown = self.shutdown.clone();
        let watcher = std::thread::Builder::new()
            .name("engram-pty-watcher".into())
            .spawn(move || loop {
                if watcher_shutdown.load(Ordering::Relaxed) {
                    return;
                }
                let exited = {
                    let mut child = sync::lock(&watcher_child);
                    matches!(child.try_wait(), Ok(Some(_)))
                };
                if exited {
                    // ADR-0001: the master drops while its lock is held (the taken value is a temporary that drops before the guard).
                    //   `shutdown()` takes this lock before publishing `master_closed`, so the pump keeps draining until this close
                    //   returns — binding the taken master and
                    //   dropping it after the guard would break that.
                    let _ = watcher_master.lock().expect("master poisoned").take();
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            })
            .expect("spawn pty watcher thread");
        drop(watcher);

        let handle = std::thread::spawn(move || {
            // ★When the pump ends the writer ends too — this guard makes that one body (the `WriterStop` doc)★; outside `contain` so drop
            //   passes in both normal and panic branches.
            let _writer_stop = writer_stop;
            // ── B-2: wrap the pump body with contain ──
            // A pump-thread panic (emit/read/try_wait) would silence that agent's output permanently with no detection or state transition
            //   (a §5 violation). Catch the body with contain; on panic make it a Failed transition via core.finish(Error), visible to user/LLM.
            //
            // UnwindSafe: the closure's captures (reader, child, shutdown, master_closed, pump_core) are unused after a panic except
            //   `pump_core`, whose finish runs outside the mark, and the thread ends right after — no logical invariant breaks;
            //   `AssertUnwindSafe`
            //   says so (the `dyn Read` reader is not `UnwindSafe`).
            // ★The capture list is **specific to this agent** — the premise of the poison scope argument below★: **resources shared by
            //   several agents** (a global registry) must not go through this closure, or this thread's panic becomes another agent's re-panic.
            //   ★Fix the list **together** when captures change★.
            // ★Mutex poison scope★: only the child lock in the reason production below is poison-tolerant (`sync::lock`) — it is this
            //   agent's own. A panic inside pump_core.emit() (holding replay/subscribers locks) poisons core Mutexes that are not
            //   poison-tolerant → the next
            //   `.expect("...poisoned")` re-panics; the finish outside the marker takes replay first, so usually the hook ends the daemon
            //   there. No real panic source exists on the emit path now (the output sink's send returns Err). The core lock stays fail-fast
            //   on purpose — poison signals possible data inconsistency, so ending beats `into_inner` and writing on.
            // ADR-0292
            let normal_reason = contain(std::panic::AssertUnwindSafe(|| {
                run_pump(
                    reader,
                    &shutdown,
                    &master_closed,
                    CLOSE_NEEDS_DRAIN,
                    |bytes| pump_core.emit(OutputEvent::TerminalBytes(bytes.to_vec())),
                    // On the kill path `shutdown`'s kill + wait may already have reaped the child, so this can be `None` — harmless
                    //   (reason `Killed`, code unused).
                    || match sync::lock(&child).try_wait() {
                        Ok(Some(status)) => Some(status.exit_code() as i32),
                        _ => None,
                    },
                )
            }));

            // ★Finalize once★: finish is called only here, outside the closure — normal and panic branches converge into one reason
            //   (normal → the closure's; panic → Error), and overlapping calls pass exactly once via OutputCore.finalized.swap(AcqRel)
            //   (ADR-0005).
            let reason = resolve_pump_reason(normal_reason);

            // ADR-0292: ★do not move finish into the closure above★ — a panic in a finish inside the marker is contained, and a re-called
            //   finish is blocked by the finalize gate, leaving a `Running` with no terminal transition (ADR-0005).
            pump_core.finish(reason);

            // G-1: completion signal for core.join_pump's recv_timeout; ignored if the receiver vanished (detached after timeout). ★Always
            //   sent on the panic path too★ (contain absorbed it) — join_pump does not stall past 5 s.
            let _ = done_tx.send(());
        });

        core.attach_pump(handle, done_rx);
    }

    /// Puts into the queue and returns **immediately** — the OS write is the writer thread's job. ★Contract · cap · 「failure after
    ///   acceptance」: the [`crate::transport::input_queue`] module header★ (not repeated).
    fn send_input(&self, input: InputEvent) -> Result<(), PtyError> {
        let InputEvent::Raw(bytes) = input;
        // ★Only accepted input raises the latch — rejected input does not raise it★ (source of record = the `input_seen` field doc).
        let accepted = self.input.push(bytes);
        if accepted.is_ok() {
            self.input_seen.store(true, Ordering::Release);
        }
        accepted
    }

    fn flush_input(&self, timeout: Duration) -> Result<(), PtyError> {
        self.input.wait_drained(timeout)
    }

    fn resize(&self, cols: u16, rows: u16) -> Result<(), PtyError> {
        if let Some(master) = self.master.lock().expect("master poisoned").as_ref() {
            master
                .resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|e| PtyError::SpawnFailed(format!("resize: {e}")))?;
        }
        Ok(())
    }

    /// ★Terminal mode does not receive the interrupt command — the terminal receives keys directly★.
    ///
    /// ★Do not substitute Ctrl-C (0x03)★: the path does not know whether a turn is running, and the claude · codex TUI can **exit** on a
    ///   second Ctrl-C when idle (likely — codex source read, claude unmeasured, ADR-0245), so the bus `agent.interrupt`
    ///   could break 「≠ kill — the process lives」 and kill the agent. A person types Esc · Ctrl-C in the terminal slot; those flow as
    ///   ordinary input (`send_input`). The rejection becomes CONFLICT on the bus.
    // ADR-0245
    fn interrupt(&self) -> Result<(), PtyError> {
        Err(PtyError::Unsupported(
            "PtyTransport::interrupt (terminal mode does not accept the interrupt command — the terminal takes keys directly)"
                .into(),
        ))
    }

    /// Resource closing stages 1~5 are in **absolute order**.
    // ADR-0001
    fn shutdown(&self) {
        // 1. shutdown signal — so the pump transitions to Killed at termination.
        self.shutdown.store(true, Ordering::Release);

        // 1b. Close the input queue — the writer sees it and ends. ★Only the queue's lock is taken, so it cannot hang★ (a blocking-write
        //   writer has released it — `InputQueue::close` doc). ★Cost: input not yet out is lost★ — the child dies on the next line anyway.
        self.input.close("the agent was terminated");

        // 2~3. wait is a reap (zombie prevention). The second call is Err since it already died — ignored (idempotent).
        {
            let mut child = self.child.lock().expect("child poisoned");
            let _ = child.kill();
            let _ = child.wait();
        }

        // 4. Terminate the whole group → down to grandchild processes → release the ConPTY slave handle. A no-op on an OS with no group.
        let _ = self.group.terminate(1);

        // 5. master.take() → drop → ClosePseudoConsole → reader EOF — the core of the kill causality. Where `CLOSE_NEEDS_DRAIN` holds, the
        //   close returns only because a running pump keeps draining until 5b (`run_pump`; with no live pump it can still block — S34 TRD §3B
        //    residual). If the watcher took the master first, this lock waits for its close (it drops under the lock).
        let _ = self.master.lock().expect("master poisoned").take();
        // 5b. Only after the statement above has completed — publishing earlier lets the pump stop while
        //     `ClosePseudoConsole` still waits for its output to be read (T-79).
        self.master_closed.store(true, Ordering::Release);
    }

    fn capabilities(&self) -> TransportCaps {
        TransportCaps {
            input: InputCaps {
                raw: true,
                message: false,
                attachment: false,
            },
            output: OutputCaps {
                terminal_bytes: true,
                structured: false,
                markdown: false,
                tool_events: false,
                usage: false,
            },
            control: ControlCaps {
                resize: true,
                // ADR-0245: the terminal receives interrupt directly — see [`Self::interrupt`].
                interrupt: false,
                cancel: false,
                graceful_shutdown: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AgentId, AgentInfo, AgentStatus, StatusSink};
    use std::sync::Mutex;

    pub(super) struct CapturingStatusSink {
        statuses: Mutex<Vec<AgentStatus>>,
    }
    impl CapturingStatusSink {
        pub(super) fn new() -> Arc<Self> {
            Arc::new(Self {
                statuses: Mutex::new(Vec::new()),
            })
        }
        pub(super) fn statuses(&self) -> Vec<AgentStatus> {
            self.statuses.lock().unwrap().clone()
        }
    }
    impl StatusSink for CapturingStatusSink {
        fn status_changed(&self, _id: AgentId, status: AgentStatus, _epoch: u32) {
            self.statuses.lock().unwrap().push(status);
        }
        fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
    }

    // ── Terminal declaration env (TERM/COLORTERM) ──

    fn spec_with_env(env: Vec<(String, String)>) -> CommandSpec {
        CommandSpec {
            program: "cmd.exe".to_string(),
            args: Vec::new(),
            env,
            cwd: std::path::PathBuf::from("."),
        }
    }

    /// ★Do not assert with `get_env`★ — it also queries the base env (`std::env::vars_os()` + Windows registry hives), so a dev shell with
    ///   TERM · COLORTERM passes **even with the injection deleted** (measured). `iter_extra_env_as_str` returns only what the
    /// caller put in. Value sentinels likewise must not be ones the environment can produce (`dumb`).
    fn extra_env(cmd: &CommandBuilder) -> Vec<(String, String)> {
        let mut entries: Vec<(String, String)> = cmd
            .iter_extra_env_as_str()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        entries.sort();
        entries
    }

    const SENTINEL_TERM: &str = "engram-sentinel-term";

    #[test]
    fn pty_command_declares_terminal_env_by_default() {
        let cmd = build_pty_command(&spec_with_env(Vec::new()));
        assert_eq!(
            extra_env(&cmd),
            vec![
                ("COLORTERM".to_string(), "truecolor".to_string()),
                ("TERM".to_string(), "xterm-256color".to_string()),
            ]
        );
    }

    #[test]
    fn profile_env_overrides_terminal_default() {
        let cmd = build_pty_command(&spec_with_env(vec![(
            "TERM".to_string(),
            SENTINEL_TERM.to_string(),
        )]));
        assert_eq!(
            extra_env(&cmd),
            vec![
                ("COLORTERM".to_string(), "truecolor".to_string()),
                ("TERM".to_string(), SENTINEL_TERM.to_string()),
            ],
            "profile keys get no default injected (exactly 1) · keys the profile did not give keep the default"
        );
    }

    /// ADR-0049 case-insensitive discipline. ★On Windows, reverting `eq_ignore_ascii_case` to `==` stays green (measured 2026-09-08)★ —
    ///   `CommandBuilder`'s map folds keys to lowercase there. The divergence is on unix, where both entries go out and make 3 —
    /// this is the net for that side. (Deleting the injection is caught too: the COLORTERM entry disappears.)
    #[test]
    fn lowercase_profile_key_suppresses_terminal_default() {
        let cmd = build_pty_command(&spec_with_env(vec![(
            "term".to_string(),
            SENTINEL_TERM.to_string(),
        )]));
        assert_eq!(
            extra_env(&cmd),
            vec![
                ("COLORTERM".to_string(), "truecolor".to_string()),
                ("term".to_string(), SENTINEL_TERM.to_string()),
            ]
        );
    }

    // ── B-2: whether the panic contain result is mapped to an Error reason ──
    #[test]
    fn resolve_pump_reason_panic_becomes_error() {
        let result = contain(std::panic::AssertUnwindSafe(|| -> TerminalReason {
            panic!("boom in pump");
        }));
        let reason = resolve_pump_reason(result);
        match reason {
            TerminalReason::Error(msg) => {
                assert!(msg.starts_with("pump panicked:"), "Error prefix: {msg}");
                assert!(
                    msg.contains("boom in pump"),
                    "original panic message preserved: {msg}"
                );
            }
            other => panic!("a panic must map to Error: {other:?}"),
        }
    }

    // ── B-2: the normal-exit reason is passed through as is ──
    #[test]
    fn resolve_pump_reason_normal_passthrough() {
        let ok = contain(std::panic::AssertUnwindSafe(|| -> TerminalReason {
            TerminalReason::Exited { code: Some(0) }
        }));
        assert!(matches!(
            resolve_pump_reason(ok),
            TerminalReason::Exited { code: Some(0) }
        ));
        let killed = contain(std::panic::AssertUnwindSafe(|| -> TerminalReason {
            TerminalReason::Killed
        }));
        assert!(matches!(
            resolve_pump_reason(killed),
            TerminalReason::Killed
        ));
    }

    // ── B-2: panic reason → core.finish → Failed transition (exactly once) ──
    #[test]
    fn panic_reason_finishes_core_as_failed_once() {
        let status_sink = CapturingStatusSink::new();
        let core = Arc::new(OutputCore::new(
            uuid::Uuid::new_v4(),
            0,
            status_sink.clone() as Arc<dyn StatusSink>,
            crate::output_core::TurnWiring::detached(),
        ));

        let panicked = contain(std::panic::AssertUnwindSafe(|| -> TerminalReason {
            panic!("simulated pump panic")
        }));
        let reason = resolve_pump_reason(panicked);
        core.finish(reason);
        // (race simulation) even if a normal EOF finish comes late, it is ignored by finalize once.
        core.finish(TerminalReason::Exited { code: Some(0) });

        let statuses = status_sink.statuses();
        assert_eq!(statuses.len(), 1, "finalize once — only 1 status change");
        match &statuses[0] {
            AgentStatus::Failed { message } => {
                assert!(
                    message.contains("pump panicked"),
                    "Failed message: {message}"
                );
            }
            other => panic!("a panic must transition to Failed: {other:?}"),
        }
        assert!(matches!(core.status(), AgentStatus::Failed { .. }));
    }

    // ── B-2: one agent's pump panic does not propagate to other agents (isolation) ──
    #[test]
    fn pump_panic_does_not_affect_other_agent() {
        let sink_a = CapturingStatusSink::new();
        let sink_b = CapturingStatusSink::new();
        let core_a = Arc::new(OutputCore::new(
            uuid::Uuid::new_v4(),
            0,
            sink_a.clone() as Arc<dyn StatusSink>,
            crate::output_core::TurnWiring::detached(),
        ));
        let core_b = Arc::new(OutputCore::new(
            uuid::Uuid::new_v4(),
            0,
            sink_b.clone() as Arc<dyn StatusSink>,
            crate::output_core::TurnWiring::detached(),
        ));

        let panicked = contain(std::panic::AssertUnwindSafe(|| -> TerminalReason {
            panic!("A panic")
        }));
        core_a.finish(resolve_pump_reason(panicked));

        core_b.emit(OutputEvent::TerminalBytes(b"alive".to_vec()));
        core_b.finish(TerminalReason::Exited { code: Some(0) });

        assert!(
            matches!(core_a.status(), AgentStatus::Failed { .. }),
            "A is Failed"
        );
        assert!(
            matches!(core_b.status(), AgentStatus::Exited { code: Some(0) }),
            "B exits normally, unaffected"
        );
        assert_eq!(sink_b.statuses().len(), 1, "B has 1 status change (normal)");
    }

    // ── run_pump: the stop rules, driven by a scripted reader ──

    /// One scripted `read` result. The `*Then*` steps raise a flag first, as a kill landing while that
    /// read is in flight would.
    enum Step {
        Bytes(&'static [u8]),
        ShutdownThenBytes(&'static [u8]),
        MasterClosedThenBytes(&'static [u8]),
        Eof,
        Fail,
    }

    /// Plays `steps` in order. A read past the end panics at once — so a loop that should have stopped
    /// fails right there instead of hanging.
    struct ScriptedReader<'a> {
        steps: std::collections::VecDeque<Step>,
        reads: usize,
        shutdown: &'a AtomicBool,
        master_closed: &'a AtomicBool,
    }

    impl Read for ScriptedReader<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.reads += 1;
            let step = self.steps.pop_front().unwrap_or_else(|| {
                panic!(
                    "read #{} is past the end of the script — the pump kept reading",
                    self.reads
                )
            });
            let bytes = match step {
                Step::Bytes(b) => b,
                Step::ShutdownThenBytes(b) => {
                    self.shutdown.store(true, Ordering::Release);
                    b
                }
                Step::MasterClosedThenBytes(b) => {
                    self.master_closed.store(true, Ordering::Release);
                    b
                }
                Step::Eof => return Ok(0),
                Step::Fail => return Err(std::io::Error::other("scripted read error")),
            };
            buf[..bytes.len()].copy_from_slice(bytes);
            Ok(bytes.len())
        }
    }

    struct PumpRun {
        reason: TerminalReason,
        emitted: Vec<Vec<u8>>,
        reads: usize,
    }

    fn pump_script(close_needs_drain: bool, steps: Vec<Step>) -> PumpRun {
        let shutdown = AtomicBool::new(false);
        let master_closed = AtomicBool::new(false);
        let mut reader = ScriptedReader {
            steps: steps.into(),
            reads: 0,
            shutdown: &shutdown,
            master_closed: &master_closed,
        };
        let mut emitted = Vec::new();
        let reason = run_pump(
            &mut reader,
            &shutdown,
            &master_closed,
            close_needs_drain,
            |bytes| emitted.push(bytes.to_vec()),
            || Some(3),
        );
        PumpRun {
            reason,
            emitted,
            reads: reader.reads,
        }
    }

    #[test]
    fn bytes_read_after_shutdown_are_not_emitted() {
        for drain in [false, true] {
            let run = pump_script(
                drain,
                vec![
                    Step::Bytes(b"before"),
                    Step::ShutdownThenBytes(b"after"),
                    Step::Bytes(b"later"),
                    Step::MasterClosedThenBytes(b"last"),
                ],
            );
            assert_eq!(
                run.emitted,
                vec![b"before".to_vec()],
                "drain = {drain}: only bytes read before shutdown may reach viewers"
            );
        }
    }

    #[test]
    fn with_drain_the_pump_keeps_reading_until_the_master_is_closed() {
        let run = pump_script(
            true,
            vec![
                Step::ShutdownThenBytes(b"a"),
                Step::Bytes(b"b"),
                Step::Bytes(b"c"),
                Step::Eof,
            ],
        );
        assert_eq!(
            run.reads, 4,
            "the pump stopped draining before the master was closed"
        );
        assert!(run.emitted.is_empty());
        assert!(matches!(run.reason, TerminalReason::Killed));
    }

    #[test]
    fn with_drain_the_first_read_after_the_master_closed_ends_the_loop_without_eof() {
        let run = pump_script(
            true,
            vec![
                Step::ShutdownThenBytes(b"a"),
                Step::Bytes(b"b"),
                Step::MasterClosedThenBytes(b"c"),
            ],
        );
        assert_eq!(run.reads, 3);
        assert!(run.emitted.is_empty());
        assert!(matches!(run.reason, TerminalReason::Killed));
    }

    #[test]
    fn without_drain_the_first_read_after_shutdown_ends_the_loop() {
        let run = pump_script(
            false,
            vec![Step::Bytes(b"a"), Step::ShutdownThenBytes(b"b")],
        );
        assert_eq!(run.reads, 2);
        assert_eq!(run.emitted, vec![b"a".to_vec()]);
        assert!(matches!(run.reason, TerminalReason::Killed));
    }

    #[test]
    fn eof_or_a_read_error_ends_the_loop() {
        for (drain, end) in [(false, Step::Eof), (true, Step::Eof), (true, Step::Fail)] {
            let run = pump_script(drain, vec![Step::Bytes(b"a"), end]);
            assert_eq!(run.reads, 2);
            assert_eq!(run.emitted, vec![b"a".to_vec()]);
        }
    }

    #[test]
    fn the_reason_is_killed_after_shutdown_and_exited_with_the_code_otherwise() {
        let exited = pump_script(true, vec![Step::Bytes(b"a"), Step::Eof]);
        assert!(matches!(
            exited.reason,
            TerminalReason::Exited { code: Some(3) }
        ));
        let killed = pump_script(true, vec![Step::ShutdownThenBytes(b"a"), Step::Eof]);
        assert!(matches!(killed.reason, TerminalReason::Killed));
    }
}

/// The wiring of the [`PtyTransport::input_seen`] latch — ★launches a real child process★ (`cmd.exe`, so `#[cfg(windows)]` and a separate
///   module from the process-free `mod tests` above). Why these two remain without a production consumer: that field's doc.
// ADR-0217
#[cfg(all(test, windows))]
mod input_seen_latch {
    use super::*;
    use crate::transport::input_queue::INPUT_QUEUE_MAX_BYTES;

    fn echo(text: &str) -> CommandSpec {
        CommandSpec {
            program: "cmd.exe".into(),
            args: vec!["/c".into(), "echo".into(), text.into()],
            env: vec![],
            cwd: std::path::PathBuf::from("."),
        }
    }

    /// ★The first accepted input raises the latch★ — without it the path can no longer say 「the child has not yet printed any user · model
    ///   content」.
    #[test]
    fn the_first_accepted_input_raises_the_latch() {
        let (transport, _pid) = PtyTransport::open(&echo("x"), 80, 24).expect("open");
        assert!(
            !transport.input_seen.load(Ordering::Acquire),
            "the latch is already raised before any input"
        );
        transport
            .send_input(InputEvent::Raw(vec![b'x']))
            .expect("within the queue cap");
        assert!(
            transport.input_seen.load(Ordering::Acquire),
            "input came in but the latch was not raised"
        );
        transport.shutdown();
    }

    /// ★The interrupt command is rejected and nothing is written to the PTY★ — if 0x03 entered the queue again it could kill an idle TUI.
    // ADR-0245
    #[test]
    fn interrupt_is_refused_and_writes_nothing() {
        let (transport, _pid) = PtyTransport::open(&echo("x"), 80, 24).expect("open");
        // ★Measure before `start()`★ — with no writer, what is in the queue is not removed so 「did not write」 is deterministic.
        assert!(
            matches!(transport.interrupt(), Err(PtyError::Unsupported(_))),
            "interrupt in terminal mode must be `Unsupported`"
        );
        assert_eq!(
            transport.input.queued_bytes(),
            0,
            "the refused interrupt left bytes in the input queue"
        );
        assert!(
            !transport.input_seen.load(Ordering::Acquire),
            "the refused interrupt raised the input latch"
        );
        assert!(
            !transport.capabilities().control.interrupt,
            "a link that refuses interrupt reports the interrupt capability"
        );
        transport.shutdown();
    }

    /// ★Rejected input does not raise the latch★ — not a single character reached the PTY so the premise stays.
    #[test]
    fn a_rejected_input_leaves_the_latch_down() {
        let (transport, _pid) = PtyTransport::open(&echo("x"), 80, 24).expect("open");
        // ★Measure before `start()`★ — with no writer, the queue is not drained so the cap decision is deterministic (the same discipline
        //   as the sibling suite
        //   `tests/transport_smoke.rs`).
        transport
            .send_input(InputEvent::Raw(vec![b'z'; INPUT_QUEUE_MAX_BYTES + 1]))
            .expect_err("exceeding the cap must be refused");
        assert!(
            !transport.input_seen.load(Ordering::Acquire),
            "the refused input raised the latch"
        );
        transport.shutdown();
        // Rejection after the queue is closed is the same.
        transport
            .send_input(InputEvent::Raw(b"late".to_vec()))
            .expect_err("a closed queue must refuse");
        assert!(
            !transport.input_seen.load(Ordering::Acquire),
            "the closed queue's refusal raised the latch"
        );
    }
}

/// ★Pins by measurement that `shutdown()` **releases a hanging ConPTY write**★.
///
/// ★Why it mattered★: `portable-pty 0.8.1`'s `take_writer()` does `Option::take` on `Inner.writable` (win/conpty.rs), so **master drop
///   does not close that write handle** — it belongs to the writer thread, which nobody joins and has no deadline; if
///   `shutdown()` could not release a hanging write, **one thread and one ConPTY handle would leak per dead agent.** Whether
///   `ClosePseudoConsole` closes the conhost-side read end is OS-private, so it could only be measured.
/// ★Measured (2026-09-18, 4/4)★: **it releases** — a write hanging with 2 MiB is released with `ERROR_BROKEN_PIPE` (os error 109) 14–17 ms
///   after `shutdown()` and the writer ends. The leak is closed; no backstop (deadline · forced handle close).
///   ★This is the regression net★ — if it breaks, reopen the backstop discussion.
///
/// ★Separate module★: production `start()` builds the write closure itself, so instrumentation cannot be inserted from outside; we take
///   `writer` out **first**, wrap it in an instrumented closure, launch the writer ourselves, then call `start()` — `writer` is
///   already `None`, so only the writer spawn is skipped and watcher · pump are as production; the measured causality (kill → Job
///   terminate → master drop) is the same.
#[cfg(all(test, windows))]
mod conpty_wedge {
    use super::*;
    use crate::output_core::TurnWiring;
    use crate::transport::input_queue::INPUT_QUEUE_MAX_BYTES;
    use crate::types::{AgentId, AgentInfo, AgentStatus, StatusSink};
    use std::sync::atomic::AtomicBool;
    use std::time::Instant;

    struct NoopStatusSink;
    impl StatusSink for NoopStatusSink {
        fn status_changed(&self, _id: AgentId, _s: AgentStatus, _e: u32) {}
        fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
    }

    #[test]
    fn shutdown_unwedges_a_blocked_conpty_write() {
        // ping does not read console input — if the ConPTY input buffer fills, our side's write hangs.
        let spec = CommandSpec {
            program: "cmd.exe".into(),
            args: vec![
                "/c".into(),
                "ping".into(),
                "-n".into(),
                "60".into(),
                "127.0.0.1".into(),
            ],
            env: vec![],
            cwd: std::path::PathBuf::from("."),
        };
        let (transport, _pid) = PtyTransport::open(&spec, 80, 24).expect("open");

        // ★Take it out **before** the production code does★ — this makes the instrumentation possible; start()'s writer spawn is skipped
        //   (module doc above).
        let mut writer = transport
            .writer
            .lock()
            .expect("writer poisoned")
            .take()
            .expect("right after open there is a writer");

        let in_write = Arc::new(AtomicBool::new(false));
        let drain_done = Arc::new(AtomicBool::new(false));
        let outcome: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        let queue = transport.input.clone();
        let w_flag = in_write.clone();
        let w_done = drain_done.clone();
        let w_out = outcome.clone();
        std::thread::Builder::new()
            .name("engram-pty-writer-instrumented".into())
            .spawn(move || {
                input_queue::drain(&queue, "PTY", AgentId::nil(), |bytes| {
                    w_flag.store(true, Ordering::Release);
                    let r = writer.write_all(bytes).and_then(|()| writer.flush());
                    w_flag.store(false, Ordering::Release);
                    w_out.lock().unwrap().push(match &r {
                        Ok(()) => format!("write ok ({} bytes)", bytes.len()),
                        Err(e) => format!("write err: {e}"),
                    });
                    r
                });
                w_done.store(true, Ordering::Release);
            })
            .expect("spawn the instrumented writer");

        let core = Arc::new(OutputCore::new(
            uuid::Uuid::new_v4(),
            0,
            Arc::new(NoopStatusSink) as Arc<dyn StatusSink>,
            TurnWiring::detached(),
        ));
        transport.start(core.clone());

        transport
            .send_input(InputEvent::Raw(vec![b'x'; INPUT_QUEUE_MAX_BYTES]))
            .expect("within the cap");

        // ① Did it really hang — this must be true for the decision below not to be vacuous.
        let enter_deadline = Instant::now() + Duration::from_secs(5);
        while !in_write.load(Ordering::Acquire) {
            assert!(
                Instant::now() < enter_deadline,
                "the writer did not even enter write within 5s — the experiment does not hold"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        std::thread::sleep(Duration::from_secs(1));
        assert!(
            in_write.load(Ordering::Acquire) && !drain_done.load(Ordering::Acquire),
            "★vacuous pass★ — {INPUT_QUEUE_MAX_BYTES} bytes went out without hanging. At this size no wedge is created, so the decision below measures nothing. Record: {:?}",
            outcome.lock().unwrap()
        );

        // ② Does shutdown release it — measured 14–17ms.
        let t0 = Instant::now();
        transport.shutdown();
        let deadline = t0 + Duration::from_secs(5);
        while !drain_done.load(Ordering::Acquire) {
            assert!(
                Instant::now() < deadline,
                "★shutdown did not release the hung write within 5s★ — every dead agent leaks a writer thread and a ConPTY write handle (in_write={}). Reopen the backstop discussion. Record: {:?}",
                in_write.load(Ordering::Acquire),
                outcome.lock().unwrap()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        core.join_pump(Duration::from_secs(5));
    }
}

/// The end of a contained pump panic — ★launches a real child process★ (separate from the process-free `mod tests`). Runs production
///   `start()`'s pump closure as is: the body runs inside the marker and `finish` outside it.
// ADR-0292
#[cfg(all(test, windows))]
mod contained_pump {
    use super::*;
    use crate::transport::pump_marks::{self, Marks};

    #[test]
    fn a_contained_pump_panic_fails_the_agent_and_finish_runs_unmarked() {
        let spec = CommandSpec {
            program: "cmd.exe".into(),
            args: vec!["/c".into(), "echo".into(), "x".into()],
            env: vec![],
            cwd: std::path::PathBuf::from("."),
        };
        let (transport, _pid) = PtyTransport::open(&spec, 80, 24).expect("open");
        let marks = Arc::new(Marks::default());
        let core = pump_marks::core(&marks);
        transport.start(core.clone());
        core.join_pump(Duration::from_secs(10));

        pump_marks::assert_contained_body_and_unmarked_finish(&marks);
        transport.shutdown();
    }
}

/// Killing mid-burst must not hang in `ClosePseudoConsole` — launches real child processes (the process-free `mod tests` stays process-free).
///
/// The producer is a grandchild with an unbounded loop, so output still flows when the kill lands. A pump that stops reading at shutdown
///   failed this in 5 of 5 runs (T-79).
// ADR-0001
#[cfg(all(test, windows))]
mod kill_mid_burst {
    use super::tests::CapturingStatusSink;
    use super::*;
    use crate::output_core::TurnWiring;
    use crate::types::{AgentStatus, StatusSink};
    use engram_dashboard_base::testing::wait_until;
    use std::time::Instant;

    const MARKER: &str = "engram-spew-line";

    struct BurstFile(std::path::PathBuf);

    impl BurstFile {
        /// A PowerShell loop writing about 2.4 MB of marker lines per call, forever. One huge write is the T-79 fixture shape (a single
        ///   1.5 MB `stdout.write`): conhost still holds most of it at group terminate, so the close has output to wait on. Line-sized producers
        /// (`for /L` with `echo` or `type`) never blocked the close on the unfixed pump (0 of 5 each).
        fn create() -> Self {
            let path =
                std::env::temp_dir().join(format!("engram-s34-burst-{}.ps1", uuid::Uuid::new_v4()));
            let script = format!(
                "$line = '{MARKER} ' + ('x' * 60) + \"`r`n\"\r\n\
                 $bytes = [Text.Encoding]::ASCII.GetBytes($line * 30000)\r\n\
                 $out = [Console]::OpenStandardOutput()\r\n\
                 while ($true) {{ $out.Write($bytes, 0, $bytes.Length) }}\r\n"
            );
            std::fs::write(&path, script).expect("write the burst file");
            Self(path)
        }
    }

    impl Drop for BurstFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn grandchild_spew(burst: &BurstFile) -> CommandSpec {
        let mut args: Vec<String> = [
            "/d",
            "/c",
            "powershell.exe",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        args.push(burst.0.to_string_lossy().into_owned());
        CommandSpec {
            program: "cmd.exe".into(),
            args,
            env: vec![],
            cwd: std::path::PathBuf::from("."),
        }
    }

    #[test]
    fn a_kill_mid_burst_returns_promptly_and_ends_killed() {
        let burst = BurstFile::create();
        let (transport, _pid) = PtyTransport::open(&grandchild_spew(&burst), 80, 24).expect("open");
        let statuses = CapturingStatusSink::new();
        let core = Arc::new(OutputCore::new(
            uuid::Uuid::new_v4(),
            0,
            statuses.clone() as Arc<dyn StatusSink>,
            TurnWiring::detached(),
        ));
        transport.start(core.clone());

        assert!(
            wait_until(Duration::from_secs(10), || {
                String::from_utf8_lossy(&core.terminal_tail(256)).contains(MARKER)
            }),
            "the spew never reached the pump within 10 s — the experiment does not stand"
        );
        std::thread::sleep(Duration::from_millis(300));

        // The kill runs on its own thread so a regression fails on the watchdog instead of hanging the
        // suite. Same two verbs as `AgentSession::kill`.
        let (done_tx, done_rx) = mpsc::channel();
        let kill_core = core.clone();
        std::thread::spawn(move || {
            let t0 = Instant::now();
            transport.shutdown();
            kill_core.join_pump(Duration::from_secs(5));
            let _ = done_tx.send(t0.elapsed());
        });
        let elapsed = done_rx.recv_timeout(Duration::from_secs(15)).expect(
            "the kill did not return within 15 s — `ClosePseudoConsole` is waiting on undrained output",
        );

        assert!(
            elapsed < Duration::from_secs(2),
            "the kill took {elapsed:?} — it must end well under the 5 s pump join"
        );
        assert!(
            statuses
                .statuses()
                .iter()
                .any(|s| matches!(s, AgentStatus::Killed)),
            "the agent must end `Killed`: {:?}",
            statuses.statuses()
        );
    }
}
