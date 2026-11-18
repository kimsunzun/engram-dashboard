//! StdioTransport — the `AgentTransport` implementation for a pipe (stdin/stdout/stderr) child: a **plain pipe process with no PTY**
//!   (unlike PtyTransport's ConPTY). Claude json mode (`-p --output-format stream-json`) is spawned here (ADR-0044);
//! terminal mode stays PtyTransport. Only the transport is swapped in the same AgentSession assembly. Zero tauri imports, zero unsafe.
//!
//! ★No-refinement invariant (ADR-0044/0045)★: this layer does not know the schema of stdout bytes — without a decoder it passes
//!   `OutputEvent::TerminalBytes` through (carrier variant), with one it only applies the injected decoder; parsing is the backend
//!   decoder's job. The interrupt line and its after-write callback ([`InterruptOut`]) are only handed to the input queue, and the group
//!   handle and retire mark (`StdioTransport::process_group`) only handed out.
//!
//! ★Difference from PTY — no watcher needed★: ConPTY gives the reader no EOF while the master lives even if the child exits (hence
//!   PtyTransport's watcher); **with a pipe the read wakes with EOF (Ok(0)) once the child tree closes all write handles** — natural
//!   exit and kill wake the pump alike.

use std::io::{Read, Write};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use engram_dashboard_base::panic::{contain, Caught};
use engram_dashboard_base::sync;
use engram_dashboard_platform::group::GroupOwner;

use crate::output_core::OutputCore;
use crate::transport::input_queue::{self, InputQueue, OnWritten};
use crate::transport::process_group::{ProcessGroup, RetiringSignal};
use crate::transport::spawn::{drain_stderr, spawn_piped, PipedChild};
use crate::transport::{AgentTransport, OutputDecoder};
use crate::types::{
    CommandSpec, ControlCaps, InputCaps, InputEvent, OutputCaps, OutputEvent, PtyError,
    TerminalReason, TransportCaps,
};

/// What the interrupt-line function gives — stdin lines plus what the writer calls after they actually go out to the pipe (contract =
///   [`OnWritten`]; discarded uncalled if they cannot go out). The path only hands both to the input queue.
pub struct InterruptOut {
    pub bytes: Vec<u8>,
    pub on_written: Option<OnWritten>,
}

/// A backend function that makes the stdin lines meaning 「stop the turn now running」; the path does not know the bytes' meaning (dumb pipe).
///
/// `None` = 「no turn to interrupt now」 — the backend's decision, passed on as `Unsupported`. A pipe has no Ctrl-C-like means of its own,
///   so interrupting is the line-maker's knowledge.
// ADR-0238
pub type InterruptLine = Arc<dyn Fn() -> Option<InterruptOut> + Send + Sync>;

pub struct StdioTransport {
    /// Shared by pump (try_wait) and shutdown (kill+wait). std Child caches the exit status after wait, so a double wait is harmless.
    child: Arc<Mutex<Child>>,
    /// ★Shared by the writer thread and `shutdown()` — unlike PTY's ownership★: handing it to the writer would leave `shutdown()` step 4's
    ///   `try_lock` cleanup nothing to reach. ★Only the writer thread holds this lock across a blocking `write_all`★, and `shutdown()`
    ///   uses only `try_lock`
    ///   (why that function's order invariant holds).
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    /// Input not yet sent. `send_input` puts it here and returns **immediately** (`transport::input_queue`).
    input: Arc<InputQueue>,
    stdout: Mutex<Option<ChildStdout>>,
    stderr: Mutex<Option<ChildStderr>>,
    shutdown: Arc<AtomicBool>,
    /// Retire mark — set by [`AgentTransport::begin_retire`] and the first line of `shutdown()`; nobody lowers it. This path does not read
    ///   it, only hands out a read-only copy via [`Self::process_group`]. An `Arc` because the handed-out copy may outlive the path.
    retiring: Arc<AtomicBool>,
    /// Whether the output this pipe carries is a structured stream (NDJSON). ★Injected (ADR-0044/0030/0191)★: it is the program's output
    ///   format (backend knowledge), not the pipe's, so the backend that makes this path passes it to `open` (no hardcoding — false for
    ///   plain-text stdio);
    /// capabilities() reports it as is.
    structured: bool,
    /// Output-refining decoder (ADR-0004/0044), injected by the backend in structured mode (none → bytes pass through = plain-text ·
    ///   terminal). Taken in start() and moved to the pump thread (None = already started).
    decoder: Mutex<Option<Box<dyn OutputDecoder>>>,
    /// Interrupt-line function. `None` = the backend gave no way to interrupt — `interrupt()` is always `Unsupported` and the capability
    ///   is false.
    // ADR-0238
    interrupt: Option<InterruptLine>,
    /// Owner of the process group this path spawned — the only one; only the weak handle of [`Self::process_group`] goes outside, so the
    ///   group closes with the path (`KILL_ON_JOB_CLOSE`). ★Keep it the last field★ — fields drop in declaration order, fixing that
    ///   moment. Moving it
    /// earlier is unmeasured and no test guards this placement.
    group: GroupOwner,
}

impl StdioTransport {
    /// **Does not spawn the pump yet** (in start). Also returns child_pid (for claude session tracking by the caller). Same signature as
    ///   PtyTransport::open minus cols/rows.
    ///
    /// ★`decoder` is an open argument, not a start argument★: injection is specific to StdioTransport, and a start argument would force
    ///   Pty/Api to take a meaningless None through the shared `AgentTransport::start`.
    pub fn open(
        spec: &CommandSpec,
        structured: bool,
        decoder: Option<Box<dyn OutputDecoder>>,
    ) -> Result<(StdioTransport, Option<u32>), PtyError> {
        let PipedChild {
            child,
            stdin,
            stdout,
            stderr,
            group,
        } = spawn_piped(spec, "stdio")?;
        let child_pid = Some(child.id());

        let transport = StdioTransport {
            child: Arc::new(Mutex::new(child)),
            stdin: Arc::new(Mutex::new(stdin)),
            input: Arc::new(InputQueue::new()),
            stdout: Mutex::new(stdout),
            stderr: Mutex::new(stderr),
            shutdown: Arc::new(AtomicBool::new(false)),
            retiring: Arc::new(AtomicBool::new(false)),
            structured,
            decoder: Mutex::new(decoder),
            interrupt: None,
            group,
        };

        Ok((transport, child_pid))
    }

    /// A weak handle to the process group this path spawned, with a read-only copy of the retire mark; the path does not know what they
    ///   are for (ADR-0044 「dumb pipe」). `None` = this OS cannot bind a group (outside Windows).
    // ADR-0262
    pub(crate) fn process_group(&self) -> Option<ProcessGroup> {
        self.group
            .downgrade()
            .map(|group| ProcessGroup::new(group, RetiringSignal::of(&self.retiring)))
    }

    /// Plugs in the interrupt-line function — then capability `control.interrupt` is true (「this path can interrupt」, not 「a turn exists
    ///   now」). Not an `open` argument, so callers without injection (plain-text stdio · tests) stay as they are.
    // ADR-0238
    pub fn with_interrupt(mut self, line: InterruptLine) -> Self {
        self.interrupt = Some(line);
        self
    }
}

/// One actual write by the writer thread. ★If `shutdown()` already took the handle, it falls through to `BrokenPipe` and
///   [`input_queue::drain`] closes the queue★ — shutting off 「input after kill silently piling up」.
fn write_stdin(stdin: &Mutex<Option<ChildStdin>>, bytes: &[u8]) -> std::io::Result<()> {
    // ★The only blocking `write_all` under this lock★ — the basis for `shutdown()`'s `try_lock` (that function's order invariant).
    let mut guard = sync::lock(stdin);
    let stdin = guard.as_mut().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::BrokenPipe, "stdin is already closed")
    })?;
    stdin.write_all(bytes)?;
    stdin.flush()
}

/// However the pump ends, it closes the input queue and reaps the writer thread (reason · basis = the same name in `pty.rs`).
struct WriterStop(Arc<InputQueue>);

impl Drop for WriterStop {
    fn drop(&mut self) {
        self.0
            .close("the stdio stream ended — nowhere left to send");
    }
}

/// A pump panic anywhere would otherwise stop that agent silently forever, so it is made visible as Failed (§5).
fn resolve_pump_reason(result: Result<TerminalReason, Caught>) -> TerminalReason {
    match result {
        Ok(reason) => reason,
        Err(caught) => TerminalReason::Error(format!("pump panicked: {caught}")),
    }
}

impl AgentTransport for StdioTransport {
    fn start(&self, core: Arc<OutputCore>) {
        let agent_id = core.id();

        let stdout = match self.stdout.lock().expect("stdout poisoned").take() {
            Some(s) => s,
            None => return,
        };

        if let Some(stderr) = self.stderr.lock().expect("stderr poisoned").take() {
            drain_stderr(stderr, &core, "engram-stdio-stderr", "stdio");
        }

        // ── input writer thread ──
        // ★Nobody joins it · it cannot delay the kill causality★ — canonical reason: the same place in `pty.rs`. Stdio-specific: it may
        //   hang in `write_stdin` holding the stdin lock, which `shutdown()`'s **kill first** order resolves into an error (that
        //   function's order invariant).
        // ★The re-call branch that could not take `stdout` does not reach here★ (it returned above), so the writer is spawned exactly once.
        {
            let queue = self.input.clone();
            let stdin = self.stdin.clone();
            let spawn_result = std::thread::Builder::new()
                .name("engram-stdio-writer".into())
                .spawn(move || {
                    // ★Without attribution (`agent = …`) this warning cannot become a trace★ — canonical reason: `input_queue::drain` doc;
                    //   same field as the spawn-failure branch below.
                    input_queue::drain(&queue, "stdio", agent_id, |bytes| {
                        write_stdin(&stdin, bytes)
                    });
                });
            if let Err(e) = spawn_result {
                // Without a writer `send_input` would return `Ok` while bytes never go out — a silent loss; close the queue and refuse
                //   honestly from then.
                let reason = format!("failed to start the stdio input writer thread: {e}");
                tracing::warn!(agent = %agent_id, "{reason}");
                self.input.close(&reason);
            }
        }

        let (done_tx, done_rx) = mpsc::channel();
        let pump_core = core.clone();
        let child = self.child.clone();
        let shutdown = self.shutdown.clone();
        // Recover even if the Mutex lock failed (poison) (avoid panic) — it is the start path, so there is no real contention.
        let mut decoder = sync::lock(&self.decoder).take();

        let writer_stop = WriterStop(self.input.clone());

        let handle = std::thread::spawn(move || {
            // ★When the pump ends, the writer ends too★ — it is outside `contain`, so both the normal and panic branches pass through.
            let _writer_stop = writer_stop;
            // ★UnwindSafe★: the captured stdout/buf/child/shutdown are dropped after a panic (the thread exits), so
            //   no logical invariant is broken → AssertUnwindSafe.
            // ADR-0292
            let normal_reason = contain(std::panic::AssertUnwindSafe(|| {
                let mut reader = stdout;
                let mut buf = [0u8; 4096];

                loop {
                    let n = match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };

                    // Auxiliary shutdown check — for when a kill lands right after read returned data.
                    //   Normally the EOF from closing the write handle wakes it first.
                    if shutdown.load(Ordering::Relaxed) {
                        break;
                    }

                    match decoder.as_mut() {
                        Some(dec) => {
                            for ev in dec.decode(&buf[..n]) {
                                pump_core.emit(ev);
                            }
                            pump_core.report_usage(dec.take_usage());
                        }
                        None => pump_core.emit(OutputEvent::TerminalBytes(buf[..n].to_vec())),
                    }
                }

                // ★Flush after break and before finish — skipped on the kill/shutdown path (FIX-2)★: the decoder may hold an unterminated
                //   tail line. On a natural EOF (the child wrote all stdout and exited) that tail may be complete data, so it is flushed
                //   before finish.
                //   ★If shutdown (kill) **cut the stream**, do not flush★: the tail is a piece truncated by our kill, and the flush (= one
                //   consume_line) could read it as parseable JSON and emit a fake event (e.g. a partial result → MessageDone). Complete
                //   newline-terminated lines
                //   were already emitted in the loop, so skipping loses no real data.
                //   ★Why branch on the shutdown flag★: natural EOF and kill are both Ok(0) at the read level, but a kill always goes
                //   through shutdown.store(Release), so an Acquire read tells them apart (same signal as the reason computation below).
                if !shutdown.load(Ordering::Acquire) {
                    if let Some(dec) = decoder.as_mut() {
                        for ev in dec.flush() {
                            pump_core.emit(ev);
                        }
                        pump_core.report_usage(dec.take_usage());
                    }
                }

                // If shutdown=true, code is unused below (Killed), so whatever the value is, it is harmless.
                let code = {
                    let mut child = sync::lock(&child);
                    match child.try_wait() {
                        Ok(Some(status)) => status.code(),
                        _ => None,
                    }
                };

                if shutdown.load(Ordering::Acquire) {
                    TerminalReason::Killed
                } else {
                    TerminalReason::Exited { code }
                }
            }));

            let reason = resolve_pump_reason(normal_reason);

            // ADR-0292: finish is outside the containment mark — do not move it into the closure (reason = the same place in `pty.rs`).
            pump_core.finish(reason);

            let _ = done_tx.send(());
        });

        core.attach_pump(handle, done_rx);
    }

    /// In json mode these bytes are already a stream-json user-turn line the backend wrapped (`{"type":"user",…}\n`) — the transport does
    ///   not know that shape (AgentSession wraps it with InputEncoder and hands it over as Raw, ADR-0044).
    /// Puts it in the queue and returns **immediately** — the OS write is the writer thread's job. ★Contract · bound · 「failure after
    ///   accepting」: the [`crate::transport::input_queue`] module header★.
    fn send_input(&self, input: InputEvent) -> Result<(), PtyError> {
        let InputEvent::Raw(bytes) = input;
        self.input.push(bytes)
    }

    fn flush_input(&self, timeout: std::time::Duration) -> Result<(), PtyError> {
        self.input.wait_drained(timeout)
    }

    fn resize(&self, _cols: u16, _rows: u16) -> Result<(), PtyError> {
        Err(PtyError::Unsupported(
            "StdioTransport::resize (a pipe has no terminal size)".into(),
        ))
    }

    /// Puts the injected function's line into the input queue — the same writer thread writes it **whole**, so lines do not mix.
    /// ★Does not take the input lock (`input_order`)★ — an interrupt is not bound to an input id, so that lock has no order to protect
    ///   here and the lock order gets no new edge. `Ok` = 「received」 as in [`Self::send_input`], not 「the turn has stopped」.
    // ADR-0238
    fn interrupt(&self) -> Result<(), PtyError> {
        let Some(line) = &self.interrupt else {
            return Err(PtyError::Unsupported(
                "StdioTransport::interrupt (this link has no interrupt line — a pipe has no Ctrl-C)"
                    .into(),
            ));
        };
        match line() {
            Some(InterruptOut { bytes, on_written }) => self.input.push_with(bytes, on_written),
            None => Err(PtyError::Unsupported(
                "StdioTransport::interrupt (no turn to interrupt)".into(),
            )),
        }
    }

    fn begin_retire(&self) {
        self.retiring.store(true, Ordering::Release);
    }

    /// The pipe version of the ADR-0001 2 verbs.
    ///
    /// ★Kill causality (pipe)★: PtyTransport wakes the pump via master drop→ConPTY close→reader EOF; with a pipe EOF comes only when **the
    ///   child tree has closed all stdout write handles**, so child.kill + Job terminate (down to the grandchild claude) is the core
    ///   (killing only cmd.exe with claude alive leaves the handles open and no EOF).
    ///
    /// ★Order invariant — stdin close must never come before kill (deadlock, FIX 1)★: the lock is now held by the **writer thread**
    ///   (`write_stdin`, across the blocking `write_all`). A child not reading stdin (pipe backpressure) blocks that write_all forever, so
    ///   `stdin.lock()` **before** the kill could never get the lock and never reach the kill → the pump cannot wake → core.join_pump
    ///   hangs (ADR-0001 stops). So **kill + Job terminate come first** — the broken pipe releases the blocked write_all with an
    ///   error and frees the lock — and only then is stdin cleaned best-effort with try_lock (never a blocking lock).
    // ADR-0001
    fn shutdown(&self) {
        // 0. Retire mark first — some ending paths skip `begin_retire`, so this keeps the window where a mark reader overlaps the
        //   termination below narrowest; the order invariant is untouched (one atomic write).
        self.retiring.store(true, Ordering::Release);

        // 1. shutdown signal — the pump transitions to Killed on exit.
        self.shutdown.store(true, Ordering::Release);

        // 1b. Close the input queue — the writer sees it and ends. ★Only the queue's lock is taken, so it cannot hang★ (a writer hanging
        //   on the stdin lock has released the queue lock — `InputQueue::close` doc); a different lock from the order invariant's (kill first).
        //     ★Cost: input not yet sent disappears★.
        self.input.close("the agent was terminated");

        // 2. wait is reap (prevents zombies). The second call is Err since it is already dead — ignored (idempotent).
        {
            let mut child = self.child.lock().expect("child poisoned");
            let _ = child.kill();
            let _ = child.wait();
        }

        // 3. End the whole group → down to the grandchild (claude under cmd). Without a group (outside Windows) it is a no-op, and
        //   child.kill kills the direct child (claude, no shim), closing the write handles.
        let _ = self.group.terminate(1);

        // 4. If try_lock fails (write_all not yet released), skip; the uncleaned ChildStdin is reclaimed on transport drop (the pipe is
        //   already broken by the kill).
        if let Ok(mut guard) = self.stdin.try_lock() {
            let _ = guard.take();
        }
    }

    fn capabilities(&self) -> TransportCaps {
        TransportCaps {
            input: InputCaps {
                raw: true,
                message: false,
                attachment: false,
            },
            output: OutputCaps {
                terminal_bytes: false,
                structured: self.structured,
                markdown: false,
                tool_events: false,
                usage: false,
            },
            control: ControlCaps {
                resize: false,
                interrupt: self.interrupt.is_some(),
                cancel: false,
                graceful_shutdown: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── resolve_pump_reason: panic → Error mapping (same rule as pty.rs) ──
    #[test]
    fn resolve_pump_reason_panic_becomes_error() {
        let result = contain(std::panic::AssertUnwindSafe(|| -> TerminalReason {
            panic!("boom in stdio pump");
        }));
        match resolve_pump_reason(result) {
            TerminalReason::Error(msg) => {
                assert!(msg.starts_with("pump panicked:"), "Error prefix: {msg}");
                assert!(
                    msg.contains("boom in stdio pump"),
                    "original message preserved: {msg}"
                );
            }
            other => panic!("a panic must map to Error: {other:?}"),
        }
    }

    #[test]
    fn resolve_pump_reason_normal_passthrough() {
        let ok = contain(std::panic::AssertUnwindSafe(|| -> TerminalReason {
            TerminalReason::Exited { code: Some(0) }
        }));
        assert!(matches!(
            resolve_pump_reason(ok),
            TerminalReason::Exited { code: Some(0) }
        ));
    }

    // ── caps honesty (FIX 2): structured is reported as the injected value (not hardcoded) + resize/interrupt false ──
    // To verify only capabilities() without a real process we need an instance → spawn a harmless child with open (echo), check caps, shutdown.
    #[cfg(windows)]
    #[test]
    fn capabilities_structured_reflects_injected_value_and_no_resize_interrupt() {
        let spec = CommandSpec {
            program: "cmd.exe".into(),
            args: vec!["/c".into(), "echo caps-probe".into()],
            env: vec![],
            cwd: std::path::PathBuf::from("."),
        };

        let (plain, _pid) = StdioTransport::open(&spec, false, None).expect("open plain");
        assert!(
            !plain.capabilities().output.structured,
            "plain stdio injected → structured=false (decided by mode, not by the pipe)"
        );
        plain.shutdown();

        let (json, _pid) = StdioTransport::open(&spec, true, None).expect("open json");
        let caps = json.capabilities();
        assert!(
            caps.output.structured,
            "json carrier injected → structured=true"
        );
        assert!(!caps.output.terminal_bytes, "not terminal bytes");
        assert!(!caps.control.resize, "a pipe cannot resize");
        assert!(
            !caps.control.interrupt,
            "a link with no interrupt line injected = no interrupt capability"
        );
        assert!(caps.input.raw, "stdin raw write possible");
        assert!(matches!(json.interrupt(), Err(PtyError::Unsupported(_))));
        assert!(matches!(json.resize(80, 24), Err(PtyError::Unsupported(_))));
        json.shutdown();
    }

    // ── Contained pump panic: in the production pump closure the body is inside the containment mark · `finish` is outside it (ADR-0292) ──
    #[cfg(windows)]
    #[test]
    fn a_contained_pump_panic_fails_the_agent_and_finish_runs_unmarked() {
        use crate::transport::pump_marks::{self, Marks};

        let spec = CommandSpec {
            program: "cmd.exe".into(),
            args: vec!["/c".into(), "echo pump-probe".into()],
            env: vec![],
            cwd: std::path::PathBuf::from("."),
        };
        let (transport, _pid) = StdioTransport::open(&spec, false, None).expect("open");
        let marks = Arc::new(Marks::default());
        let core = pump_marks::core(&marks);
        transport.start(core.clone());
        core.join_pump(std::time::Duration::from_secs(10));

        pump_marks::assert_contained_body_and_unmarked_finish(&marks);
        transport.shutdown();
    }

    // ── Interrupt line injection (ADR-0238): if the function is `Some`, that one set of lines goes into the queue with what to call · if
    //   `None`, `Unsupported` with the queue unchanged ──
    // Do not call `start()` — only without a writer can we take what is in the queue as is and measure it.
    #[cfg(windows)]
    #[test]
    fn an_injected_interrupt_line_is_queued_whole_and_a_closed_answer_is_unsupported() {
        let spec = CommandSpec {
            program: "cmd.exe".into(),
            args: vec!["/c".into(), "echo interrupt-probe".into()],
            env: vec![],
            cwd: std::path::PathBuf::from("."),
        };
        let turn_open = Arc::new(AtomicBool::new(false));
        let answer = Arc::clone(&turn_open);
        let fired = Arc::new(AtomicBool::new(false));
        let mark = Arc::clone(&fired);
        let line: InterruptLine = Arc::new(move || {
            let mark = Arc::clone(&mark);
            answer.load(Ordering::SeqCst).then(|| InterruptOut {
                bytes: b"{\"stop\":1}\n".to_vec(),
                on_written: Some(Box::new(move || mark.store(true, Ordering::SeqCst))),
            })
        });

        let (transport, _pid) = StdioTransport::open(&spec, true, None).expect("open");
        let transport = transport.with_interrupt(line);
        assert!(
            transport.capabilities().control.interrupt,
            "with an injection the capability is true — regardless of whether there is a turn now"
        );

        assert!(matches!(
            transport.interrupt(),
            Err(PtyError::Unsupported(_))
        ));
        assert_eq!(
            transport.input.queued_bytes(),
            0,
            "a refused interrupt does not touch the queue"
        );

        turn_open.store(true, Ordering::SeqCst);
        transport
            .interrupt()
            .expect("with a turn open it accepts the line");
        assert_eq!(transport.input.queued_bytes(), b"{\"stop\":1}\n".len());
        let (bytes, on_written) = transport
            .input
            .pop()
            .expect("the interrupt line in the queue");
        assert_eq!(bytes, b"{\"stop\":1}\n".to_vec());
        on_written.expect("the callback given with the line follows it into the queue")();
        assert!(fired.load(Ordering::SeqCst));
        turn_open.store(false, Ordering::SeqCst);
        assert!(
            transport.capabilities().control.interrupt,
            "the capability does not sway with the gate value"
        );
        transport.shutdown();
    }

    // ── Retire mark: `begin_retire` and `shutdown` each set it · the read-only mark the group handle handed out also sees it and
    //    stays up even after the path is gone · a notice does not reap resources ──
    // The mark is OS-independent, so it runs on all OSes — only the path is set up with a harmless child that ends soon. The group handle
    //   exists only on
    // the Windows path.
    #[test]
    fn begin_retire_and_shutdown_each_raise_the_retiring_flag() {
        #[cfg(windows)]
        let (program, args) = ("cmd.exe", vec!["/c".into(), "echo retire-probe".into()]);
        #[cfg(not(windows))]
        let (program, args) = ("true", Vec::new());
        let spec = CommandSpec {
            program: program.into(),
            args,
            env: vec![],
            cwd: std::path::PathBuf::from("."),
        };

        let (announced, _pid) = StdioTransport::open(&spec, true, None).expect("open");
        let handed = announced.process_group().map(|group| group.retiring());
        assert_eq!(
            handed.is_some(),
            cfg!(windows),
            "the group handle exists only on the Windows link"
        );
        assert!(
            !announced.retiring.load(Ordering::Acquire),
            "a freshly opened link is already retreating"
        );
        assert!(!handed.as_ref().is_some_and(|signal| signal.is_set()));
        announced.begin_retire();
        assert!(announced.retiring.load(Ordering::Acquire));
        assert!(
            handed.as_ref().is_none_or(|signal| signal.is_set()),
            "the handed-out marker does not see the notice"
        );
        announced
            .send_input(InputEvent::Raw(b"still-open\n".to_vec()))
            .expect("the notice closed the input queue — collecting is shutdown's job");
        announced.shutdown();

        let (shut, _pid) = StdioTransport::open(&spec, true, None).expect("open");
        let handed = shut.process_group().map(|group| group.retiring());
        assert!(!shut.retiring.load(Ordering::Acquire));
        assert!(!handed.as_ref().is_some_and(|signal| signal.is_set()));
        shut.shutdown();
        assert!(
            shut.retiring.load(Ordering::Acquire),
            "an exit that came without a notice does not set the retreat marker"
        );
        drop(shut);
        assert!(
            handed.as_ref().is_none_or(|signal| signal.is_set()),
            "the handed-out marker does not see an exit that came without a notice — it must stay set even after the link is gone"
        );
    }

    // ── FIX 1 regression + writer thread regression ──
    // Measures: (A) ★the caller does not hang★ — a `send_input` arriving while the writer is blocked by pipe backpressure returns
    //   immediately (else the daemon's per-connection dispatch consumer stalls on one clogged agent). (B) FIX 1 — in that
    //   blocked state `shutdown` completes without deadlock (a stdin-close-before-kill bug would never get the lock; the deadline catches it).
    // ★`start()` must be called★: what blocks is the **production writer thread**; without it nothing blocks and (A) and (B) pass vacuously.
    #[cfg(windows)]
    #[test]
    fn writer_blocks_but_send_input_returns_and_shutdown_completes() {
        use crate::output_core::TurnWiring;
        use crate::types::{AgentInfo, AgentStatus, StatusSink};
        use std::time::{Duration, Instant};

        struct NoopStatusSink;
        impl StatusSink for NoopStatusSink {
            fn status_changed(&self, _id: crate::types::AgentId, _s: AgentStatus, _e: u32) {}
            fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
        }

        // ping -n 30 = alive ~30 s and does not read stdin (a handy sleep); its small stdout is under the pipe buffer, so stdin stays
        //   unconsumed.
        let spec = CommandSpec {
            program: "cmd.exe".into(),
            args: vec![
                "/c".into(),
                "ping".into(),
                "-n".into(),
                "30".into(),
                "127.0.0.1".into(),
            ],
            env: vec![],
            cwd: std::path::PathBuf::from("."),
        };
        let (transport, _pid) = StdioTransport::open(&spec, true, None).expect("open");
        let transport = Arc::new(transport);
        let core = Arc::new(OutputCore::new(
            uuid::Uuid::new_v4(),
            0,
            Arc::new(NoopStatusSink) as Arc<dyn StatusSink>,
            TurnWiring::detached(),
        ));
        transport.start(core.clone());

        transport
            .send_input(InputEvent::Raw(vec![b'x'; 1024 * 1024]))
            .expect("below the cap, so it must be accepted");

        std::thread::sleep(Duration::from_millis(500));

        // (A) A call arriving in that blocked state returns **immediately** (the old shape would have waited for the stdin lock until the
        //   child died, ~30 s).
        let probe_start = Instant::now();
        transport
            .send_input(InputEvent::Raw(b"probe\n".to_vec()))
            .expect("there is room in the queue, so it must be accepted");
        let probe_elapsed = probe_start.elapsed();
        assert!(
            probe_elapsed < Duration::from_secs(2),
            "send_input hung for {probe_elapsed:?} while the writer was blocked — caller separation regression"
        );

        let killer = transport.clone();
        let start = Instant::now();
        let shutdown_thread = std::thread::spawn(move || killer.shutdown());

        let deadline = start + Duration::from_secs(10);
        while !shutdown_thread.is_finished() {
            assert!(
                Instant::now() < deadline,
                "shutdown did not complete within 10s — stdin lock deadlock regression (FIX 1)"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        shutdown_thread.join().expect("shutdown thread panicked");

        // Input after close is refused instead of silently piling up.
        assert!(
            matches!(
                transport.send_input(InputEvent::Raw(b"late\n".to_vec())),
                Err(PtyError::WriteFailed(_))
            ),
            "send_input after shutdown must be refused"
        );

        core.join_pump(Duration::from_secs(5));
    }

    // ── Group handle (ADR-0262): while the path lives the members are visible; once it is gone the list is empty and no one can hold it ──
    // Only drops, no `shutdown()` (shutdown ends the Job whole, emptying the list even with a strong handle). With drop alone the Job
    //   handle closes (`KILL_ON_JOB_CLOSE`) and the group ends only if the handle is weak; a held member must not hold the Job either — so
    //   the path
    // is dropped while the root and ping members are held. No `start()`, so no threads linger. cmd does not spawn ping until it gets one
    //   stdin line, so ping cannot leak out before `open` puts it into the Job.
    #[cfg(windows)]
    #[test]
    fn the_process_group_sees_the_root_and_empties_once_the_transport_is_gone() {
        use std::time::{Duration, Instant};

        // ping takes ~9 s to end by itself — ending within the 5 s wait below is the Job close's doing. Output goes to `NUL`, which also
        //   blocks the route where the pipe closes with the path and ping dies of a write failure.
        let spec = CommandSpec {
            program: "cmd.exe".into(),
            args: [
                "/d",
                "/c",
                "set",
                "/p",
                "_=",
                "&",
                "ping",
                "-n",
                "10",
                "127.0.0.1",
                ">",
                "NUL",
            ]
            .map(String::from)
            .to_vec(),
            env: vec![],
            cwd: std::path::PathBuf::from("."),
        };
        let (transport, pid) = StdioTransport::open(&spec, true, None).expect("open");
        let root_pid = pid.expect("child PID");
        let group = transport
            .process_group()
            .expect("the Windows link hands out a group");
        let root = group
            .pin(root_pid, false)
            .expect("hold on")
            .expect("the root is our member");

        // The console host attaches when cmd starts — snapshot the list and open the gate well after that, and the member that was not in
        //   that list = ping
        //   is told apart that way (we do not compare times).
        std::thread::sleep(Duration::from_millis(100));
        let before_gate = group.member_pids().expect("list before the gate");
        assert!(
            before_gate.contains(&root_pid),
            "the root is not in the list: {before_gate:?}"
        );
        {
            let mut stdin = transport.stdin.lock().unwrap_or_else(|p| p.into_inner());
            let stdin = stdin.as_mut().expect("stdin pipe");
            stdin.write_all(b"go\r\n").expect("open the gate");
            stdin.flush().expect("open the gate");
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        let ping = loop {
            let members = group.member_pids().expect("list");
            assert!(
                members.contains(&root_pid),
                "the root is not in the list: {members:?}"
            );
            let born_after_gate = members
                .into_iter()
                .filter(|pid| !before_gate.contains(pid))
                .find_map(|pid| group.pin(pid, false).ok().flatten());
            if let Some(ping) = born_after_gate {
                break ping;
            }
            assert!(
                Instant::now() < deadline,
                "did not see ping inside the Job within 10 seconds"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(!root.exited().expect("exited?") && !ping.exited().expect("exited?"));

        drop(transport);
        assert_eq!(
            group.member_pids().expect("list"),
            Vec::<u32>::new(),
            "the link is gone but the list remains — the handle is holding on to the Job"
        );
        assert!(
            group.pin(root_pid, false).expect("hold on").is_none(),
            "the link is gone but it held on to the root"
        );

        let deadline = Instant::now() + Duration::from_secs(5);
        for member in [&root, &ping] {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(
                member.wait_exit(left).expect("wait for exit"),
                "5 seconds after dropping the link the root or ping is still alive — the Job did not close: {:?}",
                member.facts()
            );
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn there_is_no_process_group_off_windows() {
        let spec = CommandSpec {
            program: "true".into(),
            args: vec![],
            env: vec![],
            cwd: std::path::PathBuf::from("."),
        };
        let (transport, _pid) = StdioTransport::open(&spec, true, None).expect("open");
        assert!(transport.process_group().is_none());
        transport.shutdown();
    }
}
