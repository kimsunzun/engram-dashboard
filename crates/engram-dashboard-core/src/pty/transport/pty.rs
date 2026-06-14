//! PtyTransport — 콘솔 백엔드(claude/codex/gemini 공용) AgentTransport 구현.
//!
//! 현 S9 코드의 세 조각을 흡수한다(동작·순서·불변식 글자 그대로):
//!   - manager.spawn_session L146-193 (openpty/spawn/job/clone_reader/take_writer) → `open`
//!   - drain.rs drain_loop + transition + spawn_drain_thread             → pump 스레드(`start`)
//!   - manager.kill_agent L463-480 (kill 1~5단계 자원 폐쇄)               → `shutdown`
//!
//! 소유권(impl-spec 표): transport는 master/writer/child/shutdown/job/reader를 소유한다.
//! cols/rows는 AgentSession이 보유하므로 여기 두지 않는다.
//!
//! tauri import 0. unsafe 0(platform/windows.rs 제외).

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

use crate::pty::output_core::OutputCore;
use crate::pty::transport::AgentTransport;
use crate::pty::types::{
    Capabilities, CommandSpec, ControlCaps, InputCaps, InputEvent, ModelCaps, OutputCaps,
    OutputEvent, PtyError, SessionCaps, TerminalReason,
};

#[cfg(windows)]
use crate::pty::platform::JobObjectHandle;

/// 콘솔 PTY transport. master/writer/child/job + start 전까지 보관하는 reader.
///
/// 소유권 분할(fable 저수준 취합 §2): child는 Arc<Mutex>로 pump(try_wait)와 shutdown(kill+wait)이
/// 공유한다. shutdown flag도 Arc — shutdown이 set(Release), pump 종료부가 read(Acquire).
pub struct PtyTransport {
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    writer: Mutex<Box<dyn Write + Send>>,
    child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
    shutdown: Arc<AtomicBool>,
    /// start()에서 take해 pump로 move. None이면 이미 시작됨.
    reader: Mutex<Option<Box<dyn Read + Send>>>,
    #[cfg(windows)]
    job_handle: JobObjectHandle,
}

impl PtyTransport {
    /// PTY 생성 + child spawn + job 편입 + reader/writer 확보. **pump는 아직 안 띄운다.**
    /// 현 manager.spawn_session L146-193을 순서 그대로 옮긴 것. child_pid를 함께 반환한다
    /// (claude 세션 추적 부착용 — 호출자가 사용).
    pub fn open(
        spec: &CommandSpec,
        cols: u16,
        rows: u16,
    ) -> Result<(PtyTransport, Option<u32>), PtyError> {
        // 1. PTY 생성.
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| PtyError::SpawnFailed(format!("openpty: {e}")))?;

        // 2. child spawn (program + args + cwd + env). PtyTransport는 claude/codex를 모른다 —
        //    backend가 산출한 CommandSpec만 본다.
        let mut cmd = CommandBuilder::new(&spec.program);
        for a in &spec.args {
            cmd.arg(a);
        }
        cmd.cwd(&spec.cwd);
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| PtyError::SpawnFailed(format!("spawn: {e}")))?;

        // slave는 spawn 후 불필요 — drop으로 FD 누수 방지(닫혀야 ConPTY EOF도 정상).
        drop(pair.slave);

        let child_pid = child.process_id();

        // 3. Windows: Job 생성 + child 편입 (spike/windows.rs 검증 순서 그대로).
        #[cfg(windows)]
        let job_handle = {
            let job = JobObjectHandle::new()?;
            if let Some(pid) = child_pid {
                job.assign(pid)?;
            }
            job
        };

        // 4. ★master를 적재하기 전에 reader/writer를 먼저 확보★.
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| PtyError::SpawnFailed(format!("clone_reader: {e}")))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| PtyError::SpawnFailed(format!("take_writer: {e}")))?;

        // 5. 필드 적재. reader는 start()에서 take할 때까지 보관.
        let transport = PtyTransport {
            master: Mutex::new(Some(pair.master)),
            writer: Mutex::new(writer),
            child: Arc::new(Mutex::new(child)),
            shutdown: Arc::new(AtomicBool::new(false)),
            reader: Mutex::new(Some(reader)),
            #[cfg(windows)]
            job_handle,
        };

        Ok((transport, child_pid))
    }
}

impl AgentTransport for PtyTransport {
    /// pump 스레드 기동 + core 연결. reader가 이미 take됐으면(재호출) 아무것도 안 한다.
    ///
    /// pump 스레드는 drain.rs의 drain_loop + transition + spawn_drain_thread를 흡수한 것이다.
    fn start(&self, core: Arc<OutputCore>) {
        // reader take — 없으면 이미 시작됨(멱등 방어).
        let reader = match self.reader.lock().expect("reader poisoned").take() {
            Some(r) => r,
            None => return,
        };

        // G-1 완료 채널 + 공유 자원 Arc 확보(pump가 core/child/shutdown을 만진다).
        // core는 clone해 pump로 move — 원본은 아래 attach_pump에 쓴다.
        let (done_tx, done_rx) = mpsc::channel();
        let pump_core = core.clone();
        let child = self.child.clone();
        let shutdown = self.shutdown.clone();

        let handle = std::thread::spawn(move || {
            let mut reader = reader;
            let mut buf = [0u8; 4096];

            // ── drain_loop (drain.rs L44-100) ──
            loop {
                // 1. blocking read — read 자체가 자연 배칭. EOF(master drop) 또는 Err로 깨면 종료.
                let n = match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };

                // 2. shutdown 보조 확인(현 drain.rs step2 안전망 그대로). 보통 master drop EOF가
                //    먼저 깨우지만, read가 데이터를 막 반환한 직후 kill이 걸린 경우를 위한 안전망.
                if shutdown.load(Ordering::Relaxed) {
                    break;
                }

                // 3. core가 seq 발급·replay·fanout을 전담(불변식 1·2는 OutputCore::emit 안).
                pump_core.emit(OutputEvent::TerminalBytes(buf[..n].to_vec()));
            }

            // ── transition (drain.rs L110-136)의 reason 산출 ──
            // child exit code를 status lock 무관하게 try_wait로 취득. Killed 경로면 안 쓰지만
            // (lock 안에서 판정이 갈리던 현 구조와 동치로) 미리 확보해 둔다.
            // 주: kill 경로는 shutdown의 child.kill()+wait()가 이미 reap했을 수 있어 None일 수 있다
            //     — 그땐 shutdown=true라 Killed로 가므로 code 미사용, 무해.
            let code = {
                let mut child = child.lock().expect("child poisoned");
                match child.try_wait() {
                    Ok(Some(status)) => Some(status.exit_code() as i32),
                    _ => None,
                }
            };

            // shutdown store(Release)와 페어링되도록 Acquire로 읽는다(현 drain.rs와 동일).
            // 이미 terminal이면 덮지 않는 idempotent 규칙은 core.finish의 finalize 게이트가 담당.
            let reason = if shutdown.load(Ordering::Acquire) {
                TerminalReason::Killed
            } else {
                TerminalReason::Exited { code }
            };

            // terminal 알림 주체 = pump(=core.finish). finalize 정확히 1회.
            pump_core.finish(reason);

            // G-1: 완료 신호. core.join_pump의 recv_timeout가 받는다. 수신측이 이미
            // 사라졌어도(타임아웃 후 detach) 무시.
            let _ = done_tx.send(());
        });

        // pump 핸들/done_rx를 core에 적재 — kill 6단계(core.join_pump)가 이걸 쓴다.
        core.attach_pump(handle, done_rx);
    }

    /// 입력 전달 — 현 manager.write_stdin L409-419. Raw 바이트만 처리.
    fn send_input(&self, input: InputEvent) -> Result<(), PtyError> {
        match input {
            InputEvent::Raw(bytes) => {
                let mut writer = self.writer.lock().expect("writer poisoned");
                writer
                    .write_all(&bytes)
                    .map_err(|e| PtyError::WriteFailed(e.to_string()))?;
                writer
                    .flush()
                    .map_err(|e| PtyError::WriteFailed(e.to_string()))?;
                Ok(())
            }
        }
    }

    /// PTY cols/rows 변경 — 현 manager.resize L422-433의 master.resize 부분.
    /// cols/rows의 atomic 저장은 여기서 안 한다(AgentSession 책임).
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

    /// interrupt ≠ kill. 0x03(Ctrl+C)을 stdin으로 주입해 진행 중 작업만 중단한다 —
    /// 프로세스는 살아 있다(kill은 shutdown). send_input(Raw) 경로 재사용.
    fn interrupt(&self) -> Result<(), PtyError> {
        self.send_input(InputEvent::Raw(vec![0x03]))
    }

    /// 자원 강제 종료 — 현 manager.kill_agent L463-480의 1~5단계 절대순서. **멱등.**
    /// pump 종료 대기(recv)는 여기서 안 한다 — core.join_pump 몫(2동사 분리).
    fn shutdown(&self) {
        // 1. shutdown 신호 — pump가 종료 시 Killed로 전이하도록(store Release, pump가 Acquire).
        self.shutdown.store(true, Ordering::Release);

        // 2~3. child kill + wait(reap, 좀비 방지). 두 번째 호출은 이미 죽었으니 Err — 무시(멱등).
        {
            let mut child = self.child.lock().expect("child poisoned");
            let _ = child.kill();
            let _ = child.wait();
        }

        // 4. Windows: Job 전체 종료 → 손자 프로세스까지 → ConPTY slave 핸들 해제.
        #[cfg(windows)]
        {
            let _ = self.job_handle.terminate(1);
        }

        // 5. master.take() → drop → ClosePseudoConsole → reader EOF.
        //    take는 멱등(두 번째는 None). 이 drop이 pump read를 EOF로 깨운다 — 인과의 핵심.
        let _ = self.master.lock().expect("master poisoned").take();
    }

    /// 콘솔 capability — terminal-bytes 단방향 출력, raw 입력, resize/interrupt 가능.
    /// resume=true(claude --resume), snapshot/model/graceful_shutdown은 미지원.
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            input: InputCaps {
                raw: true,
                message: false,
                attachment: false,
            },
            output: OutputCaps {
                terminal_bytes: true,
                markdown: false,
                tool_events: false,
                usage: false,
            },
            control: ControlCaps {
                resize: true,
                interrupt: true,
                cancel: false,
                graceful_shutdown: false,
            },
            session: SessionCaps {
                resume: true,
                snapshot: false,
                cwd_env: true,
            },
            model: ModelCaps {
                select: false,
                temperature: false,
                max_tokens: false,
            },
        }
    }
}
