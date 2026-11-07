//! StdioTransport — the `AgentTransport` implementation for a pipe (stdin/stdout/stderr) child process.
//!
//! Unlike PtyTransport (ConPTY interactive), it is a **plain pipe process with no PTY**. Claude json mode
//! (`-p --output-format stream-json`, headless) is spawned by this transport (ADR-0044). Terminal mode stays
//! PtyTransport. In the same AgentSession assembly, only the transport is swapped.
//!
//! ★No-refinement invariant (ADR-0044/0045)★: the transport layer does not know the schema of stdout bytes — with no decoder
//!   it passes them through as `OutputEvent::TerminalBytes` (reusing the carrier variant), and with one it only applies the injected
//!   decoder. Parsing is the backend decoder's job — not this layer's, not the frontend's.
//!   The interrupt line and what to call after that line goes out ([`InterruptOut`]) are also only handed to the input queue, and the group handle and retire mark
//!   (`StdioTransport::process_group`) are only handed out — the backend knows how they are used.
//!
//! ★Key difference from PTY — no watcher needed★: with ConPTY, if the master is alive the reader gets no EOF even when the child exits by itself,
//!   so PtyTransport has a watcher thread to detect natural exit. **With a pipe,
//!   the read wakes with EOF (Ok(0)) once the child (and the child tree) closes all write handles** — natural exit and
//!   kill alike wake the pump the same way, so there is no separate watcher (simpler by that much).
//!
//! tauri import 0. unsafe 0.

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

/// What the interrupt-line function gives — one set of lines to write to stdin, and what the writer calls after that line actually goes out to the pipe (contract =
/// [`OnWritten`] — if it cannot go out it is not called and is discarded). The path only hands both to the input queue.
pub struct InterruptOut {
    pub bytes: Vec<u8>,
    pub on_written: Option<OnWritten>,
}

/// A backend function that makes one set of stdin lines meaning 「지금 도는 턴을 멈춰 달라」 (stop the turn now running). The path does not know the meaning of those bytes (dumb pipe).
///
/// `None` = 「지금은 끊을 턴이 없다」 (there is no turn to interrupt now) — the decision is the backend's, and the path passes it on as `Unsupported` as is. A pipe has
/// no interrupt means of the path's own like Ctrl-C, so interrupting is knowledge of whoever makes this line.
// ADR-0238
pub type InterruptLine = Arc<dyn Fn() -> Option<InterruptOut> + Send + Sync>;

pub struct StdioTransport {
    /// Shared by pump (try_wait) and shutdown (kill+wait). std Child caches the exit status after wait, so
    /// even if shutdown reaps first, the pump's try_wait retrieves the same status (double wait is harmless).
    child: Arc<Mutex<Child>>,
    /// ★Shared by the writer thread and `shutdown()` — unlike the PTY side, which goes with ownership★. If this handle were
    ///   handed to the writer, the `try_lock` cleanup of `shutdown()` step 4 would have nowhere to reach. So
    ///   it is shared, but ★the only one that now holds this lock with a blocking `write_all` is the writer thread★ and `shutdown()`
    ///   still uses only `try_lock` (why the order invariant of that function remains valid).
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    /// Input not yet sent out. `send_input` puts it here and returns **immediately** (module = `transport::input_queue`).
    input: Arc<InputQueue>,
    /// take in start() and move to the pump thread. None means already started.
    stdout: Mutex<Option<ChildStdout>>,
    /// take in start() and move to the drain thread.
    stderr: Mutex<Option<ChildStderr>>,
    /// shutdown (kill) progress signal. If set (Release), the pump transitions to Killed at exit (the pump reads with Acquire).
    shutdown: Arc<AtomicBool>,
    /// Retire mark — set by [`AgentTransport::begin_retire`] and the first line of `shutdown()`, and no one lowers it. This path does
    /// not read it, and only hands it out as a read-only copy carried in [`Self::process_group`] — the path does not know what it is used for. It is an `Arc` because
    /// the handed-out copy may outlive the path (the field is held jointly).
    retiring: Arc<AtomicBool>,
    /// Whether the output this pipe carries is a structured stream (NDJSON). ★An injected value (ADR-0044/0030/0191)★:
    /// "is it structured" is decided not by the pipe but by the output format of that program (backend knowledge), so the backend that makes this path
    /// injects it as an `open` argument (no hardcoding — false for plain-text stdio). capabilities() reports it as is.
    structured: bool,
    /// Output-refining decoder (ADR-0004/0044). The backend that makes this path injects it in structured mode (if none,
    /// bytes pass straight through = the plain-text · terminal path). Taken in start() and moved to the pump thread (= None means already started).
    decoder: Mutex<Option<Box<dyn OutputDecoder>>>,
    /// Interrupt-line function. `None` = the backend that made this path did not give a way to interrupt — `interrupt()` is always `Unsupported` and
    /// the capability is false as well.
    // ADR-0238
    interrupt: Option<InterruptLine>,
    /// Owner of the process group this path spawned — there is only one, and only the weak handle of [`Self::process_group`] is handed outside, so
    /// when the path goes away the group closes too (`KILL_ON_JOB_CLOSE`). ★Keep it as the last field★ — fields are dropped in declaration order, so that
    /// closing moment is fixed by this position. Whether moving it earlier is safe has not been measured, and no test guards this placement.
    group: GroupOwner,
}

impl StdioTransport {
    /// **Does not spawn the pump yet** (in start). Also returns child_pid (for attaching claude session tracking
    /// — used by the caller). Matches the signature of PtyTransport::open, except there is no cols/rows.
    ///
    /// ★Why `decoder` is taken as an open argument, not a start argument★: decoder injection is specific to StdioTransport,
    /// so it does not touch the shared `AgentTransport::start` signature (if it did, Pty/Api would also have to take a
    /// meaningless None argument — minimizing the ripple).
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

    /// A weak handle to the process group this path spawned — it also carries a read-only copy of the retire mark. The path does not know what
    /// it is used for (ADR-0044 「바보 파이프」). `None` = this OS has no means of binding a group (outside Windows).
    // ADR-0262
    pub(crate) fn process_group(&self) -> Option<ProcessGroup> {
        self.group
            .downgrade()
            .map(|group| ProcessGroup::new(group, RetiringSignal::of(&self.retiring)))
    }

    /// Plugs in the interrupt-line function — once plugged in, the capability `control.interrupt` becomes true (meaning 「끊을 수 있는 통로다」 (this is a path that can interrupt), not 「지금 턴이 있다」 (a turn exists now)).
    /// It is not an `open` argument so that callers without injection (plain-text stdio · tests) keep standing as they do today.
    // ADR-0238
    pub fn with_interrupt(mut self, line: InterruptLine) -> Self {
        self.interrupt = Some(line);
        self
    }
}

/// One actual write by the writer thread. ★If `shutdown()` has already taken the handle away, it falls through to `BrokenPipe` and
/// [`input_queue::drain`] closes the queue★ — that combination shuts off the branch of 「kill 뒤에 남은 입력이 조용히 쌓이는」 (input left after kill silently piling up).
fn write_stdin(stdin: &Mutex<Option<ChildStdin>>, bytes: &[u8]) -> std::io::Result<()> {
    // ★The only place that does a blocking `write_all` under this lock★ — the basis for `shutdown()` using `try_lock`
    //   (the order invariant of that function) points here.
    let mut guard = sync::lock(stdin);
    let stdin = guard.as_mut().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::BrokenPipe, "stdin 이 이미 닫혔다")
    })?;
    stdin.write_all(bytes)?;
    stdin.flush()
}

/// However the pump ends, it closes the input queue and reaps the writer thread (reason · basis = the same name in `pty.rs`).
struct WriterStop(Arc<InputQueue>);

impl Drop for WriterStop {
    fn drop(&mut self) {
        self.0.close("stdio 스트림이 끝났다 — 더 보낼 곳이 없다");
    }
}

/// If the pump thread panics anywhere, that agent would be silently stopped forever, so it is made visible as Failed (§5).
fn resolve_pump_reason(result: Result<TerminalReason, Caught>) -> TerminalReason {
    match result {
        Ok(reason) => reason,
        Err(caught) => TerminalReason::Error(format!("pump panicked: {caught}")),
    }
}

impl AgentTransport for StdioTransport {
    /// If stdout was already taken (re-call), do nothing (idempotence defense).
    fn start(&self, core: Arc<OutputCore>) {
        let agent_id = core.id();

        let stdout = match self.stdout.lock().expect("stdout poisoned").take() {
            Some(s) => s,
            None => return,
        };

        // ── stderr drain thread ──
        if let Some(stderr) = self.stderr.lock().expect("stderr poisoned").take() {
            drain_stderr(stderr, &core, "engram-stdio-stderr", "stdio");
        }

        // ── input writer thread ──
        // ★No one joins it · it cannot delay the kill causality★ — the canonical reason is the same place in `pty.rs`.
        //   One fact only in this stdio version: this thread may hang inside `write_stdin` holding the stdin lock,
        //   and the order in which `shutdown()` does **kill first** resolves that into an error (the order invariant of that function).
        // ★The re-call branch that could not take `stdout` does not reach here★ — it already returned above.
        //   So the writer is also spawned exactly once.
        {
            let queue = self.input.clone();
            let stdin = self.stdin.clone();
            let spawn_result = std::thread::Builder::new()
                .name("engram-stdio-writer".into())
                .spawn(move || {
                    // ★Without attribution (`agent = …`) this warning cannot become a trace★ — the canonical reason is
                    //   the doc of `input_queue::drain`. The same field as the spawn-failure branch below.
                    input_queue::drain(&queue, "stdio", agent_id, |bytes| {
                        write_stdin(&stdin, bytes)
                    });
                });
            if let Err(e) = spawn_result {
                // Without a writer, `send_input` returns `Ok` while the bytes never go out — a silent
                //   loss, so close the queue and refuse honestly from that moment.
                let reason = format!("stdio 입력 라이터 스레드 기동 실패: {e}");
                tracing::warn!(agent = %agent_id, "{reason}");
                self.input.close(&reason);
            }
        }

        // ── pump thread (stdout→core) ──
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

                // ★Flush after break and before finish — but skipped on the kill/shutdown path (FIX-2)★:
                //   the decoder may hold in its buffer a residual tail line that ended without a newline. If it is a natural EOF
                //   (the child wrote out all of stdout and exited normally), this tail may be complete data that actually arrived to the end,
                //   so it is spat out before finish to prevent loss.
                //   ★But if the stream was **cut midway** by shutdown (kill), do not flush★:
                //   that tail is a piece truncated because we killed the process, so it is not a "complete line".
                //   This cuts off at the source any chance that the flush (= one consume_line) reads that truncated piece by accident as parseable JSON and
                //   emits a fake event (e.g. a partial result → MessageDone).
                //   Complete lines that actually arrived and ended with a newline were already all emitted by the decode inside the loop, so
                //   skipping loses no real data (what remains in the buffer is only an unterminated tail).
                //   ★Why branch on the shutdown flag★: at the read level, natural EOF and kill are both
                //   Ok(0) and cannot be told apart, but a kill always goes through shutdown.store(Release), so
                //   reading it here with Acquire tells them apart for certain (the same signal source as the reason computation below).
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

            // G-1: completion signal (received by core.join_pump's recv_timeout). Ignore it even if the receiver is gone.
            let _ = done_tx.send(());
        });

        core.attach_pump(handle, done_rx);
    }

    /// In json mode these bytes are already a stream-json user-turn line the backend wrapped
    /// (`{"type":"user",…}\n`) — the transport does not know that shape
    /// (AgentSession wraps it with InputEncoder and hands it over as Raw, ADR-0044 isolation).
    /// Puts it in the queue and returns **immediately** — the OS write is the job of the dedicated writer thread.
    ///
    /// ★The canonical source of the contract · the bound · 「받아 둔 뒤의 실패」 (failure after accepting) is the header of the [`crate::transport::input_queue`] module★.
    fn send_input(&self, input: InputEvent) -> Result<(), PtyError> {
        let InputEvent::Raw(bytes) = input;
        self.input.push(bytes)
    }

    fn flush_input(&self, timeout: std::time::Duration) -> Result<(), PtyError> {
        self.input.wait_drained(timeout)
    }

    fn resize(&self, _cols: u16, _rows: u16) -> Result<(), PtyError> {
        Err(PtyError::Unsupported(
            "StdioTransport::resize (파이프는 터미널 크기 없음)".into(),
        ))
    }

    /// Puts the line given by the injected function into the input queue — the same writer thread as for user lines writes it **whole**, so lines do not mix.
    /// ★It does not take the input lock (`input_order`)★ — an interrupt is not bound to an input id, so there is no order for that lock to protect, and so
    ///   there is no new edge in the lock order. `Ok` means 「받았다」 (received), as with [`Self::send_input`], and not 「턴이 멈췄다」 (the turn has stopped).
    // ADR-0238
    fn interrupt(&self) -> Result<(), PtyError> {
        let Some(line) = &self.interrupt else {
            return Err(PtyError::Unsupported(
                "StdioTransport::interrupt (이 통로엔 끊기 줄이 없다 — 파이프엔 Ctrl-C 가 없다)"
                    .into(),
            ));
        };
        match line() {
            Some(InterruptOut { bytes, on_written }) => self.input.push_with(bytes, on_written),
            None => Err(PtyError::Unsupported(
                "StdioTransport::interrupt (끊을 턴이 없다)".into(),
            )),
        }
    }

    fn begin_retire(&self) {
        self.retiring.store(true, Ordering::Release);
    }

    /// The pipe version of the ADR-0001 2 verbs.
    ///
    /// ★Kill causality (pipe version)★: PtyTransport wakes the pump via master drop→ConPTY close→reader EOF, but
    ///   with a pipe the reader wakes with EOF only when **the child tree has closed all stdout write handles**. So
    ///   killing the whole tree with child.kill + Job terminate (down to the grandchild claude) and closing the write handles is
    ///   the core of the causality (if only cmd.exe is killed and claude stays alive, the write handles do not close and no EOF comes).
    ///
    /// ★Order invariant — stdin close must never come before kill (deadlock, FIX 1)★:
    ///   ★It once read 「send_input 은 …」 (send_input is …) here — only the holder of that lock changed, and the danger is the same★.
    ///   Now what holds that lock is the **writer thread** (`write_stdin`, throughout the blocking `write_all`). If the child
    ///   does not read stdin (pipe backpressure), that write_all blocks forever and never releases the lock. If we then try to close it with
    ///   `stdin.lock()` **before** the kill, we can never get that lock, so we cannot even reach the kill → the pump
    ///   cannot wake → core.join_pump hangs forever (the ADR-0001 causality stops). So **kill + Job
    ///   terminate come first**: killing the child breaks the pipe, so the blocked write_all is released with an error and the
    ///   lock is freed. Only after that is stdin cleaned up best-effort with try_lock (a blocking lock is never allowed).
    /// ※graceful-exit-via-stdin-close is not needed — we kill here anyway.
    // ADR-0001
    fn shutdown(&self) {
        // 0. Retire mark first of all — there are ending paths that do not go through `begin_retire`, so the window where a reader of the mark overlaps the
        //    termination below is kept narrowest here too. The order invariant below is untouched (it is a single atomic write).
        self.retiring.store(true, Ordering::Release);

        // 1. shutdown signal — the pump transitions to Killed on exit.
        self.shutdown.store(true, Ordering::Release);

        // 1b. Close the input queue — the writer thread sees this and ends. ★What we take here is **only the queue's lock**, so
        //     it cannot hang★: a writer hanging on the stdin lock has already released the queue lock (`InputQueue::close` doc).
        //     So this one line does not touch the order invariant below (kill first) — it is a different lock.
        //     ★Cost = input not yet sent out disappears★.
        self.input.close("에이전트를 종료했다");

        // 2. wait is reap (prevents zombies). The second call is Err since it is already dead — ignored (idempotent).
        {
            let mut child = self.child.lock().expect("child poisoned");
            let _ = child.kill();
            let _ = child.wait();
        }

        // 3. End the whole group → down to the grandchild (claude under cmd). On an OS with no group (outside Windows) it is a no-op, and child.kill
        //    kills the direct child (claude, no shim), which closes the write handles.
        let _ = self.group.terminate(1);

        // 4. If try_lock cannot be obtained (the instant write_all has not yet been released), just skip. The uncleaned ChildStdin is
        //    reclaimed by the OS on transport drop, so no leak (the pipe was already broken by the kill).
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
                assert!(msg.contains("boom in stdio pump"), "원 메시지 보존: {msg}");
            }
            other => panic!("panic 은 Error 로 매핑돼야: {other:?}"),
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
    // To verify only capabilities() without a real process we need an instance → spawn a harmless child with open
    // (echo, exits immediately), check only caps, and clean up with shutdown.
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
            "평문 stdio 주입 → structured=false(파이프가 아니라 mode 가 결정)"
        );
        plain.shutdown();

        let (json, _pid) = StdioTransport::open(&spec, true, None).expect("open json");
        let caps = json.capabilities();
        assert!(caps.output.structured, "json 캐리어 주입 → structured=true");
        assert!(!caps.output.terminal_bytes, "터미널 바이트 아님");
        assert!(!caps.control.resize, "파이프 resize 불가");
        assert!(
            !caps.control.interrupt,
            "끊기 줄을 주입하지 않은 통로 = 끊기 능력 없음"
        );
        assert!(caps.input.raw, "stdin raw 쓰기 가능");
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

    // ── Interrupt line injection (ADR-0238): if the function is `Some`, that one set of lines goes into the queue with what to call · if `None`, `Unsupported` with the queue unchanged ──
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
            "주입이 있으면 능력은 참 — 지금 턴이 있나와 무관하다"
        );

        assert!(matches!(
            transport.interrupt(),
            Err(PtyError::Unsupported(_))
        ));
        assert_eq!(
            transport.input.queued_bytes(),
            0,
            "거절한 끊기는 큐를 건드리지 않는다"
        );

        turn_open.store(true, Ordering::SeqCst);
        transport.interrupt().expect("턴이 열려 있으면 줄을 받는다");
        assert_eq!(transport.input.queued_bytes(), b"{\"stop\":1}\n".len());
        let (bytes, on_written) = transport.input.pop().expect("큐에 든 끊기 줄");
        assert_eq!(bytes, b"{\"stop\":1}\n".to_vec());
        on_written.expect("줄과 함께 준 부를 것이 큐까지 따라온다")();
        assert!(fired.load(Ordering::SeqCst));
        turn_open.store(false, Ordering::SeqCst);
        assert!(
            transport.capabilities().control.interrupt,
            "능력은 문 값을 따라 흔들리지 않는다"
        );
        transport.shutdown();
    }

    // ── Retire mark: `begin_retire` and `shutdown` each set it · the read-only mark the group handle handed out also sees it and
    //    stays up even after the path is gone · a notice does not reap resources ──
    // The mark is OS-independent, so it runs on all OSes — only the path is set up with a harmless child that ends soon. The group handle exists only on
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
            "무리 손잡이는 Windows 통로에만 있다"
        );
        assert!(
            !announced.retiring.load(Ordering::Acquire),
            "갓 연 통로가 이미 물러나는 중이다"
        );
        assert!(!handed.as_ref().is_some_and(|signal| signal.is_set()));
        announced.begin_retire();
        assert!(announced.retiring.load(Ordering::Acquire));
        assert!(
            handed.as_ref().is_none_or(|signal| signal.is_set()),
            "내준 표시가 예고를 못 본다"
        );
        announced
            .send_input(InputEvent::Raw(b"still-open\n".to_vec()))
            .expect("예고가 입력 큐를 닫았다 — 거두기는 shutdown 몫이다");
        announced.shutdown();

        let (shut, _pid) = StdioTransport::open(&spec, true, None).expect("open");
        let handed = shut.process_group().map(|group| group.retiring());
        assert!(!shut.retiring.load(Ordering::Acquire));
        assert!(!handed.as_ref().is_some_and(|signal| signal.is_set()));
        shut.shutdown();
        assert!(
            shut.retiring.load(Ordering::Acquire),
            "예고 없이 온 종료가 물러남 표시를 세우지 않는다"
        );
        drop(shut);
        assert!(
            handed.as_ref().is_none_or(|signal| signal.is_set()),
            "예고 없이 온 종료를 내준 표시가 못 본다 — 통로가 사라진 뒤에도 선 채여야 한다"
        );
    }

    // ── FIX 1 regression + writer thread regression ──
    // Measures (two):
    //   (A) ★The caller does not hang★ — a `send_input` arriving **in the very state** where the writer is blocked by pipe backpressure
    //       returns immediately. This is the property this round established, and without it the daemon's
    //       per-connection dispatch consumer would stall entirely on one clogged agent.
    //   (B) FIX 1 as is — in that blocked state `shutdown` completes without deadlock. If the bug (= stdin close before
    //       kill) were there, it could never get that lock and would hang, and the deadline below catches it.
    // ★Where it differs from the old shape★: before, a thread the test spawned itself blocked inside `send_input`. Now
    //   what blocks is the **production writer thread**, so `start()` must be called — if not, there is no writer and
    //   nothing blocks, and both (A) and (B) pass vacuously.
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

        // ping -n 30 = stays alive for ~30s and does not read stdin (a handy sleep). A small amount of stdout is
        // under the pipe buffer, so the child does not block on stdout either → stdin stays unconsumed.
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

        // 1MiB = far beyond the pipe buffer · under the queue cap (2MiB). The writer bites into this and blocks.
        transport
            .send_input(InputEvent::Raw(vec![b'x'; 1024 * 1024]))
            .expect("상한 아래라 받아야 한다");

        // Yield briefly (generously) so the writer surely enters write_all and takes the stdin lock.
        std::thread::sleep(Duration::from_millis(500));

        // (A) The call arriving in that blocked state returns **immediately**. With the old shape it would have waited here for the stdin lock,
        //     hanging until the child died (~30s).
        let probe_start = Instant::now();
        transport
            .send_input(InputEvent::Raw(b"probe\n".to_vec()))
            .expect("큐에 자리가 있으므로 받아야 한다");
        let probe_elapsed = probe_start.elapsed();
        assert!(
            probe_elapsed < Duration::from_secs(2),
            "라이터가 막힌 동안 send_input 이 {probe_elapsed:?} 매달렸다 — 호출자 분리 회귀"
        );

        // (B) FIX 1 — in the same state shutdown completes without deadlock.
        let killer = transport.clone();
        let start = Instant::now();
        let shutdown_thread = std::thread::spawn(move || killer.shutdown());

        let deadline = start + Duration::from_secs(10);
        while !shutdown_thread.is_finished() {
            assert!(
                Instant::now() < deadline,
                "shutdown 이 10s 안에 완료되지 않음 — stdin 락 데드락 회귀(FIX 1)"
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
            "shutdown 뒤의 send_input 은 거절돼야 한다"
        );

        core.join_pump(Duration::from_secs(5));
    }

    // ── Group handle (ADR-0262): while the path lives the members are visible, and once the path is gone the list is empty and no one can hold it ──
    // Only drops, with no `shutdown()` — shutdown ends the Job whole, so the list is empty even if the handle is strong. With drop
    // alone, the Job handle closes (`KILL_ON_JOB_CLOSE`) and the group ends only if the handle is weak; if strong, ping stays in the list. A held
    // member must not hold the Job either for the group to end — so the path is dropped while the root and ping members are held.
    // Since `start()` is not called the path has no threads, so nothing is left hanging after the drop.
    // cmd does not spawn ping until it receives one line of stdin — so that ping does not leak out before `open` puts it into the Job.
    #[cfg(windows)]
    #[test]
    fn the_process_group_sees_the_root_and_empties_once_the_transport_is_gone() {
        use std::time::{Duration, Instant};

        // ping takes ~9 seconds to end by itself — if it ends within the 5-second wait below, that is the doing of the Job close. Output goes to `NUL`, so
        //   it also blocks the route where the pipe closes as the path goes away and ping dies from a write failure.
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
        let root_pid = pid.expect("자식 PID");
        let group = transport
            .process_group()
            .expect("Windows 통로는 무리를 내준다");
        let root = group
            .pin(root_pid, false)
            .expect("붙들기")
            .expect("뿌리는 우리 멤버다");

        // The console host attaches when cmd starts — snapshot the list and open the gate well after that, and the member that was not in that list = ping
        //   is told apart that way (we do not compare times).
        std::thread::sleep(Duration::from_millis(100));
        let before_gate = group.member_pids().expect("문 앞 명단");
        assert!(
            before_gate.contains(&root_pid),
            "뿌리가 명단에 없다: {before_gate:?}"
        );
        {
            let mut stdin = transport.stdin.lock().unwrap_or_else(|p| p.into_inner());
            let stdin = stdin.as_mut().expect("stdin 파이프");
            stdin.write_all(b"go\r\n").expect("문 열기");
            stdin.flush().expect("문 열기");
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        let ping = loop {
            let members = group.member_pids().expect("명단");
            assert!(
                members.contains(&root_pid),
                "뿌리가 명단에 없다: {members:?}"
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
                "10 초 안에 Job 안의 ping 을 못 봤다"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(!root.exited().expect("끝났나") && !ping.exited().expect("끝났나"));

        drop(transport);
        assert_eq!(
            group.member_pids().expect("명단"),
            Vec::<u32>::new(),
            "통로가 사라졌는데 명단이 남았다 — 손잡이가 Job 을 붙들고 있다"
        );
        assert!(
            group.pin(root_pid, false).expect("붙들기").is_none(),
            "통로가 사라졌는데 뿌리를 붙들었다"
        );

        let deadline = Instant::now() + Duration::from_secs(5);
        for member in [&root, &ping] {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(
                member.wait_exit(left).expect("끝나기 대기"),
                "통로를 버린 뒤 5 초가 지나도 뿌리나 ping 이 살아 있다 — Job 이 안 닫혔다: {:?}",
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
