//! PtyTransport — the AgentTransport implementation shared by console backends (claude/codex/gemini).
//!
//! tauri import 0. unsafe 0.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use engram_dashboard_base::panic::{contain, Caught};
use engram_dashboard_base::sync;
use engram_dashboard_platform::group::GroupOwner;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

use crate::output_core::OutputCore;
use crate::transport::input_queue::{self, InputQueue};
use crate::transport::spawn::{new_group_with, ChildGuard};
use crate::transport::AgentTransport;
use crate::types::{
    CommandSpec, ControlCaps, InputCaps, InputEvent, OutputCaps, OutputEvent, PtyError,
    TerminalReason, TransportCaps,
};

/// Ownership split (fable low-level roundup §2): child is shared by the pump (try_wait) and shutdown (kill+wait) as Arc<Mutex>.
/// The shutdown flag is an Arc too — shutdown does set (Release), the pump's termination part does read (Acquire).
pub struct PtyTransport {
    /// master can be dropped (take) by both the watcher (natural-exit detection) and shutdown (kill), so it is shared by Arc.
    /// Both are `take()` so it is idempotent — the side that takes first closes the ConPTY, and the later side sees None.
    master: Arc<Mutex<Option<Box<dyn MasterPty + Send>>>>,
    /// ★The writer thread **owns** it — taken in `start()` and moved, and after that this is `None`★.
    ///
    /// ★Why owned and not shared (`Arc<Mutex<…>>`)★: `portable_pty`'s writer **can be taken only once**
    ///   (trait doc: "It is invalid to take the writer more than once" — unix does `bail!`, Windows
    ///   ConPTY does `Option::take()`) and is `Send` but not `Sync`. That is, this handle must belong to **one thread**,
    ///   and that thread is the writer. This is why it differs from the stdio side sharing via `Arc<Mutex<Option<ChildStdin>>>`
    ///   (there the `try_lock` cleanup discipline of `shutdown()` requires sharing the handle).
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    /// Input that has not gone out yet. `send_input` puts into it and returns **immediately** (module = `transport::input_queue`).
    input: Arc<InputQueue>,
    child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
    shutdown: Arc<AtomicBool>,
    /// Taken in start() and moved to the pump. If None, already started.
    reader: Mutex<Option<Box<dyn Read + Send>>>,
    /// ★Has this path ever **accepted** input — the latch of 「자식이 아직 사용자·모델 내용을 찍은 적이 없다」
    /// (the child has not yet printed any user · model content)★.
    ///
    /// ★Today the only place reading this value is tests — still do not delete it★: the fact it carries can be known only by the path
    ///   (this is the only place that sees both the input side and the output side) and cannot be rebuilt. The only production
    ///   consumer, the pre-input observation window, was withdrawn by ADR-0217.
    /// ★Raise it the moment the queue **accepts** — not the moment the OS write finishes★: the fact to be kept shakes from the moment
    ///   it enters the queue (we do not know when the writer will write). If raised late, bytes read in between
    ///   would be wrongly counted as 「입력 전」 (before input).
    /// ★**Do not raise** it for rejected input★ — bytes the queue sent back because it was closed or over the cap never
    ///   reach the PTY, so the premise stays (exceeding the cap is a branch that can actually be reached —
    ///   `tests/transport_smoke.rs`).
    /// ★Write is `Release`★ — the reading side must receive with `Acquire` so order stands between 「입력을 받아들였다」 (input was accepted) and the side that observes it.
    ///   If left `Relaxed`, that claim would lean on an accidental barrier.
    input_seen: Arc<AtomicBool>,
    /// The owner of the process group this path launched. ★Put it as the last field★ — fields are dropped in declaration order so the moment the group closes when the path is dropped
    /// (the OS ends the remaining members) is fixed by this spot. We have not re-measured whether moving it earlier is allowed, and no test guards this
    /// arrangement.
    group: GroupOwner,
}

impl PtyTransport {
    /// **The pump is not launched yet** (in start). It also returns child_pid
    /// (for attaching claude session tracking — used by the caller).
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
        // ★The first step that can fail after launch = putting into the group · declare the guard before the group★ — if a step below fails, the local variables are
        //   dropped in reverse declaration order (group → guard). Reason = the `new_group_with` doc.
        let child_pid = guard.pid();
        let group = new_group_with(child_pid)?;

        // The slave is unneeded after spawn — drop prevents an FD leak (it must be closed for ConPTY EOF to be normal too).
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
            reader: Mutex::new(Some(reader)),
            input_seen: Arc::new(AtomicBool::new(false)),
            group,
        };

        Ok((transport, child_pid))
    }
}

/// Why it was split out of open(): the default-value injection rule for the terminal-declaration env is load-bearing, so it is made verifiable directly in a unit test
/// without a real spawn.
fn build_pty_command(spec: &CommandSpec) -> CommandBuilder {
    let mut cmd = CommandBuilder::new(&spec.program);
    for a in &spec.args {
        cmd.arg(a);
    }
    cmd.cwd(&spec.cwd);
    // ADR-0184: the PTY transport declares a terminal identity to the child (the four rejected alternatives = that ADR).
    // The daemon is launched through WMI (WmiPrvSE.exe is the parent), so the inherited environment is the **service environment** and TERM · COLORTERM
    //   are absent altogether. portable-pty puts the whole inherited environment (+ on Windows the HKLM/HKCU `Environment` hives) as is
    //   into the base env but **does not synthesize TERM** (the only thing it synthesizes is unix's SHELL).
    //   Without it the codex TUI loses the lower border of the header box and the prompt background block (measured 2026-09-08 —
    //   giving just these two variables makes it identical to Windows Terminal). This layer that hands out the PTY is the terminal itself,
    //   so regardless of backend it asserts "what terminal we are" here.
    // ★Inherited values are overridden★ — the child draws onto our xterm.js widget, not onto whatever launched the daemon. The inherited
    //   TERM does not describe that widget so the default wins. The only thing it yields to is the profile (spec.env).
    // It follows ADR-0049's env default-value discipline as is — ★explicit-skip★: if the profile already gives the same key, it does not
    //   inject (a deterministic way that does not lean on merge-order last-wins). ★Case-insensitive★: Windows
    //   environment variables are case-insensitive, and on unix a lowercase `term` does not override `TERM` but **both**
    //   go out in the child block — eq_ignore_ascii_case blocks that branch.
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
/// ★The point is that it is a `Drop`★ — whether it ends normally by EOF or the pump body panics (that branch is absorbed by `contain`), this
///   spot is always passed. It is the same discipline as the codex path's `ReaderExit` and is needed for the same reason:
///   ★if missed, **a writer thread remains forever for every naturally exited agent**★. `shutdown()` is not called from the reaper path
///   so there is no other place to wake that thread, and it stays holding the queue and the PTY writer handle.
struct WriterStop(Arc<InputQueue>);

impl Drop for WriterStop {
    fn drop(&mut self) {
        self.0.close("PTY 스트림이 끝났다 — 더 보낼 곳이 없다");
    }
}

/// B-2. Why it was split out of the pump closure: this mapping is load-bearing (panic→Failed transition), so it is made verifiable directly in a unit test
/// without a real PTY child.
fn resolve_pump_reason(result: Result<TerminalReason, Caught>) -> TerminalReason {
    match result {
        Ok(reason) => reason,
        Err(caught) => TerminalReason::Error(format!("pump panicked: {caught}")),
    }
}

impl AgentTransport for PtyTransport {
    /// If the reader was already taken (a re-call), it does nothing.
    fn start(&self, core: Arc<OutputCore>) {
        let reader = match self.reader.lock().expect("reader poisoned").take() {
            Some(r) => r,
            None => return,
        };

        // ── Input writer thread ──
        // ★Nobody joins it★ — `core` holds only one pump handle (widening that spot is a core change),
        //   and waiting inside `shutdown()` violates ADR-0001's 2-verb contract. This thread ends by itself when the queue closes
        //   (the `Drop` of `WriterStop` and `shutdown()` are those two), and if there is a hanging `write_all`,
        //   kill+master drop releases it as an error. ★So this thread **cannot delay the kill causality**★.
        // ★Why it is launched in `start()` (= not in `open()`)★: put in the same place as the codex writer to separate 「통로를 만든
        //   것」 (what made the path) from 「통로를 돌리기 시작한 것」 (what started running the path). Input that came before `start()` stands in the queue as is and
        //   flows out here — the caller need not know whether it is ready (ADR-0190 discipline 1).
        if let Some(mut writer) = self.writer.lock().expect("writer poisoned").take() {
            let queue = self.input.clone();
            // ★To attach attribution to the warnings the writer leaves, the id must be passed here★ — inside it
            //   `core` cannot be held (only the queue and the writer are moved to the thread). It uses the same field the spawn failure
            //   branch below already leaves as `agent = %core.id()`.
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
                // ★Not left silent★: if there is no writer, `send_input` keeps returning `Ok` while bytes
                //   never go out — a silent loss itself. Close the queue and **reject honestly from that moment**.
                let reason = format!("PTY 입력 라이터 스레드 기동 실패: {e}");
                tracing::warn!(agent = %core.id(), "{reason}");
                self.input.close(&reason);
            }
        }

        let (done_tx, done_rx) = mpsc::channel();
        let pump_core = core.clone();
        let child = self.child.clone();
        let shutdown = self.shutdown.clone();
        let writer_stop = WriterStop(self.input.clone());

        // ── Natural-exit detection watcher (console only — this detection is kept inside PtyTransport only) ──
        // Problem: on Windows ConPTY, as long as the master is alive, even if the child exits by itself it does not give the reader EOF.
        //   So on natural exit (cmd /c exit) the pump's blocking read never wakes
        //   → core.finish is not called → the reaper signal does not go. (On the kill path, shutdown drops the master so
        //   EOF comes and it is normal.) To compensate: poll to detect child termination and **drop the master**, so that
        //   it takes the existing EOF→pump break→finish→reaper path as is.
        //
        // ★The shutdown flag is not touched★ — if set, the pump transitions to Killed. A natural exit
        //   must be yielded exactly as Exited{code} for status · logs to be right (since ADR-0083 the reaper disposition is
        //   KeepAsIs only for shutdown and KeepDisableAutoRestore for everything else, so deletion is unaffected, but distinguishing the exit code is
        //   still needed for status/diagnostics), so the watcher only drops the master. The reason is produced by the pump from try_wait's
        //   exit code.
        //
        // ★Deadlock/double-wait safety★: WinChild::try_wait/wait/kill try_clone the internal proc handle and
        //   release the outer Mutex immediately, so the watcher holds our child Mutex **only briefly** (one try_wait)
        //   and sleeps → even if it races with shutdown's child.lock()+kill+wait it is released soon and no deadlock.
        //   Windows has no zombie reaping and holding the handle = preserving the exit code, so it is harmless for the watcher's try_wait and
        //   the pump's later try_wait to retrieve the same code repeatedly (no double-reap problem).
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
                    let _ = watcher_master.lock().expect("master poisoned").take();
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            })
            .expect("spawn pty watcher thread");
        // The watcher handle is detached (not joined) — it returns soon either way, kill or natural exit.
        drop(watcher);

        let handle = std::thread::spawn(move || {
            // ★When the pump ends the writer ends too — this guard makes that one body (the `WriterStop` doc).★
            //   Put outside `contain` so drop passes in both the normal and the panic branch.
            let _writer_stop = writer_stop;
            // ── B-2: wrap the pump body with contain ──
            // If the pump thread panics (anywhere in emit/read/try_wait) that agent's output goes permanently silent
            // and stops, and there was no detection or state transition (a §5 violation). Catch the body with contain, and on panic
            // make it a Failed transition with core.finish(Error) and make it visible to the user/LLM.
            //
            // ★UnwindSafe★: the reader/buf/child/shutdown that the closure captures are not used any more after a panic and are
            //   dropped (the thread ends soon), so no logical invariant is broken → stated explicitly with AssertUnwindSafe.
            //   The Mutex(child) itself is UnwindSafe but the capture bundle (especially the dyn Read reader) is not, so we wrap it.
            // ★That the capture list is **specific to this agent** is the premise of the poison scope argument below★ — so
            //   **resources shared by several agents** (like a global registry) must not be sent through this closure.
            //   If that resource's lock is released by `expect`, this thread's panic becomes another agent's
            //   re-panic.
            // ★Fix this list **together** when the captures change★ — a stale list makes the poison scope argument above
            //   a statement that cannot be verified.
            // ★Mutex poison scope (precisely)★: only the child lock in the reason production below is handled poison-tolerant
            //   (`sync::lock`) — child is specific to this transport (= this agent), so its poison
            //   does not propagate to other agents.
            //   However, if a panic blows up inside pump_core.emit() (while holding the replay/subscribers lock), those
            //   core Mutexes are poisoned and not poison-tolerant → the next .expect("...poisoned")
            //   that takes that lock re-panics. The finish outside the marker right below takes replay first, so
            //   usually the hook ends the daemon there. There is currently no real panic source on the emit path (the daemon output sink's
            //   send returns Err instead of panicking). So the core lock intentionally stays fail-fast (expect) —
            //   poison is a signal of "possible data inconsistency" so ending is safer than ignoring it (into_inner) and writing on.
            // ADR-0292
            let normal_reason = contain(std::panic::AssertUnwindSafe(|| {
                let mut reader = reader;
                let mut buf = [0u8; 4096];

                loop {
                    // blocking read — the read itself is natural batching (no separate batch layer).
                    let n = match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };

                    // shutdown auxiliary check — usually master drop EOF wakes first, but this is a safety net for the case where kill hits
                    //   right after read just returned data.
                    if shutdown.load(Ordering::Relaxed) {
                        break;
                    }

                    pump_core.emit(OutputEvent::TerminalBytes(buf[..n].to_vec()));
                }

                // Not used on the Killed path but secured in advance.
                // Note: on the kill path shutdown's child.kill()+wait() may have already reaped so it may be None
                //     — then shutdown=true so it goes to Killed and the code is unused, harmless.
                let code = {
                    let mut child = sync::lock(&child);
                    match child.try_wait() {
                        Ok(Some(status)) => Some(status.exit_code() as i32),
                        _ => None,
                    }
                };

                if shutdown.load(Ordering::Acquire) {
                    TerminalReason::Killed
                } else {
                    TerminalReason::Exited { code }
                }
            }));

            // ★Preserve finalize once★: finish is called only at this one place outside the closure — the normal and panic branches
            //   converge into one reason (normal → the reason the closure produced · panic → Error), and even if they are called overlapping,
            //   OutputCore.finalized.swap(AcqRel) lets exactly one through (ADR-0005).
            let reason = resolve_pump_reason(normal_reason);

            // ADR-0292: ★do not move finish into the closure above★ — a panic in a finish made inside the marker is contained, and a re-called
            //   finish is blocked by the finalize gate and leaves a `Running` with no terminal transition (ADR-0005). Here it is outside the marker.
            pump_core.finish(reason);

            // G-1: completion signal. Received by core.join_pump's recv_timeout. Ignored even if the receiving side
            // has already vanished (detach after timeout). ★Always sent on the panic path too★ —
            // contain absorbed the panic so this send is reached (join_pump does not stall past 5s).
            let _ = done_tx.send(());
        });

        core.attach_pump(handle, done_rx);
    }

    /// Puts into the queue and returns **immediately** — the OS write is the dedicated writer thread's job.
    ///
    /// ★The source of record for the contract · cap · 「받아 둔 뒤의 실패」 (failure after being accepted) is the [`crate::transport::input_queue`] module header★. It is
    ///   not repeated here.
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

    /// ★Terminal mode does not receive the interrupt command — the terminal receives keys directly★ (user decision 2026-09-28).
    ///
    /// ★Do not put Ctrl-C (0x03) in on its behalf★: the path does not know whether a turn is running, and the claude · codex TUI can **exit** on a second
    ///   Ctrl-C when idle (likely — read from the codex source · claude unmeasured, ADR-0245) — the bus
    ///   `agent.interrupt` could break the contract 「≠ kill — 프로세스는 산다」 (≠ kill — the process lives) and kill the agent. A person who wants to interrupt types Esc · Ctrl-C directly in the terminal slot, and those keys flow as ordinary input
    ///   (`send_input`). The rejection is translated to CONFLICT on the bus.
    // ADR-0245
    fn interrupt(&self) -> Result<(), PtyError> {
        Err(PtyError::Unsupported(
            "PtyTransport::interrupt (터미널 모드는 끊기 명령을 받지 않는다 — 터미널이 키를 직접 받는다)"
                .into(),
        ))
    }

    /// Resource closing stages 1~5 are in **absolute order**.
    // ADR-0001
    fn shutdown(&self) {
        // 1. shutdown signal — so the pump transitions to Killed at termination.
        self.shutdown.store(true, Ordering::Release);

        // 1b. Close the input queue — the writer thread sees this and ends. ★All that is taken here is the queue's lock, so
        //     it cannot hang★ (a writer in a blocking write has released that lock — the `InputQueue::close` doc).
        //     ★Cost = input that has not gone out yet is lost★: this child dies on the next line so writing the rest would fail anyway.
        self.input.close("에이전트를 종료했다");

        // 2~3. wait is a reap (zombie prevention). The second call is Err since it already died — ignored (idempotent).
        {
            let mut child = self.child.lock().expect("child poisoned");
            let _ = child.kill();
            let _ = child.wait();
        }

        // 4. Terminate the whole group → down to grandchild processes → release the ConPTY slave handle. A no-op on an OS with no group.
        let _ = self.group.terminate(1);

        // 5. master.take() → drop → ClosePseudoConsole → reader EOF — the core of the causality.
        let _ = self.master.lock().expect("master poisoned").take();
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

    struct CapturingStatusSink {
        statuses: Mutex<Vec<AgentStatus>>,
    }
    impl CapturingStatusSink {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                statuses: Mutex::new(Vec::new()),
            })
        }
        fn statuses(&self) -> Vec<AgentStatus> {
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

    /// ★Do not assert with `get_env`★ — it also queries the base env (= `std::env::vars_os()` + the Windows registry
    /// hives), so in a dev shell where TERM · COLORTERM already exist it passes **even if the injection code is deleted**
    /// (measured). `iter_extra_env_as_str` returns only items the caller put in directly, measuring "this code
    /// put it in". Value sentinels are the same — using a value the surrounding environment can accidentally produce (`dumb`, etc.)
    /// makes the profile-priority assertion meaningless again.
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
            "프로필 키는 기본값 미주입(정확히 1개) · 프로필이 안 준 키는 기본값 유지"
        );
    }

    /// ADR-0049 case-insensitive discipline. ★On Windows, even if `eq_ignore_ascii_case` is changed back to `==`
    /// this test is green (measured 2026-09-08)★ — because `CommandBuilder`'s internal map on Windows folds keys to
    /// lowercase so `TERM`/`term` merge into one entry anyway. The divergence is on unix,
    /// where both entries go out in the child block and become 3 — this assertion is the regression net for that side.
    /// (A regression that deletes the injection itself is caught here too, as the COLORTERM entry disappears.)
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
                    "원래 panic 메시지 보존: {msg}"
                );
            }
            other => panic!("panic 은 Error 로 매핑돼야: {other:?}"),
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
        assert_eq!(statuses.len(), 1, "finalize 1회 — status 변경 1건만");
        match &statuses[0] {
            AgentStatus::Failed { message } => {
                assert!(
                    message.contains("pump panicked"),
                    "Failed 메시지: {message}"
                );
            }
            other => panic!("panic 은 Failed 로 전이해야: {other:?}"),
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
            "A 는 Failed"
        );
        assert!(
            matches!(core_b.status(), AgentStatus::Exited { code: Some(0) }),
            "B 는 영향 없이 정상 Exited"
        );
        assert_eq!(sink_b.statuses().len(), 1, "B status 변경 1건(정상)");
    }
}

/// The wiring of the [`PtyTransport::input_seen`] latch — ★it launches a real child process★.
///
/// ★It is a separate module because of `#[cfg(windows)]`★ — it launches `cmd.exe` so it does not hold on other platforms. The `mod tests` above
///   launches no process at all as a property of that module, so they are not mixed.
/// ★The source of record for why these two remain despite having no production consumer = that field's doc★.
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

    /// ★The first accepted input raises the latch★ — without it the path can no longer say 「자식이 아직 사용자·모델 내용을 찍은 적이
    /// 없다」 (the child has not yet printed any user · model content).
    #[test]
    fn the_first_accepted_input_raises_the_latch() {
        let (transport, _pid) = PtyTransport::open(&echo("x"), 80, 24).expect("open");
        assert!(
            !transport.input_seen.load(Ordering::Acquire),
            "입력 전인데 빗장이 이미 올라가 있다"
        );
        transport
            .send_input(InputEvent::Raw(vec![b'x']))
            .expect("큐 상한 이내");
        assert!(
            transport.input_seen.load(Ordering::Acquire),
            "입력이 들어왔는데 빗장이 안 올라갔다"
        );
        transport.shutdown();
    }

    /// ★The interrupt command is rejected and nothing is written to the PTY★ — if 0x03 entered the queue again it could kill an idle TUI.
    // ADR-0245
    #[test]
    fn interrupt_is_refused_and_writes_nothing() {
        let (transport, _pid) = PtyTransport::open(&echo("x"), 80, 24).expect("open");
        // ★Measure before `start()`★ — with no writer, what is in the queue is not removed so 「안 썼다」 (did not write) is deterministic.
        assert!(
            matches!(transport.interrupt(), Err(PtyError::Unsupported(_))),
            "터미널 모드 끊기는 `Unsupported` 여야 한다"
        );
        assert_eq!(
            transport.input.queued_bytes(),
            0,
            "거절한 끊기가 입력 큐에 바이트를 남겼다"
        );
        assert!(
            !transport.input_seen.load(Ordering::Acquire),
            "거절한 끊기가 입력 빗장을 올렸다"
        );
        assert!(
            !transport.capabilities().control.interrupt,
            "끊기를 거절하는 통로가 끊기 능력을 신고한다"
        );
        transport.shutdown();
    }

    /// ★Rejected input does not raise the latch★ — not a single character reached the PTY so the premise stays.
    #[test]
    fn a_rejected_input_leaves_the_latch_down() {
        let (transport, _pid) = PtyTransport::open(&echo("x"), 80, 24).expect("open");
        // ★Measure before `start()`★ — with no writer, the queue is not drained so the cap decision is deterministic (the same discipline as the sibling suite
        //   `tests/transport_smoke.rs`).
        transport
            .send_input(InputEvent::Raw(vec![b'z'; INPUT_QUEUE_MAX_BYTES + 1]))
            .expect_err("상한 초과는 거절돼야 한다");
        assert!(
            !transport.input_seen.load(Ordering::Acquire),
            "거절당한 입력이 빗장을 올렸다"
        );
        transport.shutdown();
        // Rejection after the queue is closed is the same.
        transport
            .send_input(InputEvent::Raw(b"late".to_vec()))
            .expect_err("닫힌 큐는 거절돼야 한다");
        assert!(
            !transport.input_seen.load(Ordering::Acquire),
            "닫힌 큐의 거절이 빗장을 올렸다"
        );
    }
}

/// ★The spot that pins by measurement that `shutdown()` **releases a hanging ConPTY write**★.
///
/// ★Why this question mattered★: `portable-pty 0.8.1`'s `take_writer()` does `Option::take` on `Inner.writable`
///   (win/conpty.rs), so **master drop does not close that write handle** — the handle belongs to the writer
///   thread. Nobody joins that thread and there is no deadline, so if `shutdown()` could not release a hanging write,
///   **one thread and one ConPTY handle would leak permanently for every dead agent.** The only remaining candidate was
///   whether `ClosePseudoConsole` closes the conhost-side read end, and that is OS-private behavior so
///   it could not be inferred and could only be measured.
/// ★Measured conclusion (2026-09-18, 4/4 reproduced)★: **it releases.** A write hanging while holding 2MiB is released with
///   `ERROR_BROKEN_PIPE` (os error 109) 14–17ms after `shutdown()` and the writer ends by itself. So the leak concern is
///   closed, and we do not build a backstop (deadline · forced handle close). ★This item is the regression net for that conclusion★ — if it breaks,
///   the leak above has revived so the backstop discussion must be reopened.
///
/// ★Why a separate module★: the production `start()` builds the write closure itself so instrumentation cannot be inserted from outside.
///   Instead we take `writer` out **first**, wrap it in an instrumented closure, launch the writer ourselves, and then call `start()`
///   — in `start()` `writer` is already `None` so only the writer spawn is skipped and the watcher · pump are set up as is, so
///   the causality `shutdown()` measures (kill → Job terminate → master drop) is the same as production.
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

        // ★Take it out **before** the production code does★ — this one line makes the instrumentation possible, and at that price start()'s writer
        //   spawn is skipped (the module doc above).
        let mut writer = transport
            .writer
            .lock()
            .expect("writer poisoned")
            .take()
            .expect("open 직후엔 writer 가 있다");

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
            .expect("계측 라이터 spawn");

        let core = Arc::new(OutputCore::new(
            uuid::Uuid::new_v4(),
            0,
            Arc::new(NoopStatusSink) as Arc<dyn StatusSink>,
            TurnWiring::detached(),
        ));
        transport.start(core.clone());

        transport
            .send_input(InputEvent::Raw(vec![b'x'; INPUT_QUEUE_MAX_BYTES]))
            .expect("상한 이내");

        // ① Did it really hang — this must be true for the decision below not to be vacuous.
        let enter_deadline = Instant::now() + Duration::from_secs(5);
        while !in_write.load(Ordering::Acquire) {
            assert!(
                Instant::now() < enter_deadline,
                "라이터가 5s 안에 write 에 진입조차 못했다 — 실험이 성립하지 않는다"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        std::thread::sleep(Duration::from_secs(1));
        assert!(
            in_write.load(Ordering::Acquire) && !drain_done.load(Ordering::Acquire),
            "★공허한 통과★ — {INPUT_QUEUE_MAX_BYTES} 바이트가 매달리지 않고 그냥 나갔다. 이 크기로는              wedge 가 만들어지지 않으므로 아래 판정이 아무것도 재지 못한다. 기록: {:?}",
            outcome.lock().unwrap()
        );

        // ② Does shutdown release it — measured 14–17ms.
        let t0 = Instant::now();
        transport.shutdown();
        let deadline = t0 + Duration::from_secs(5);
        while !drain_done.load(Ordering::Acquire) {
            assert!(
                Instant::now() < deadline,
                "★shutdown 이 매달린 write 를 5s 안에 풀지 못했다★ — 죽은 에이전트마다 라이터 스레드와                  ConPTY 쓰기 핸들이 샌다(in_write={}). 백스톱 논의를 다시 열 것. 기록: {:?}",
                in_write.load(Ordering::Acquire),
                outcome.lock().unwrap()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        core.join_pump(Duration::from_secs(5));
    }
}

/// The end of a contained pump panic — ★it launches a real child process★ (the `mod tests` above is a module that launches no process so they are not mixed).
/// It runs the production `start()`'s pump closure as is and measures that the body runs inside the marker and `finish` runs outside the marker.
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
