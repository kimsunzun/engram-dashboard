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
use std::time::Duration;

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

use crate::agent::output_core::OutputCore;
use crate::agent::transport::AgentTransport;
use crate::agent::types::{
    CommandSpec, ControlCaps, InputCaps, InputEvent, OutputCaps, OutputEvent, PtyError,
    TerminalReason, TransportCaps,
};

#[cfg(windows)]
use crate::agent::platform::JobObjectHandle;

/// 콘솔 PTY transport. master/writer/child/job + start 전까지 보관하는 reader.
///
/// 소유권 분할(fable 저수준 취합 §2): child는 Arc<Mutex>로 pump(try_wait)와 shutdown(kill+wait)이
/// 공유한다. shutdown flag도 Arc — shutdown이 set(Release), pump 종료부가 read(Acquire).
pub struct PtyTransport {
    /// master 는 watcher(자연 종료 감지)와 shutdown(kill) 둘 다 drop(take)할 수 있어 Arc 공유한다.
    /// 둘 다 `take()` 라 멱등 — 먼저 take 한 쪽이 ConPTY 를 닫고, 나중 쪽은 None 을 본다.
    master: Arc<Mutex<Option<Box<dyn MasterPty + Send>>>>,
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
            master: Arc::new(Mutex::new(Some(pair.master))),
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

/// catch_unwind 결과 → 종료 reason 매핑(B-2). 정상이면 본체가 산출한 reason 을 그대로,
/// panic 이면 payload 에서 메시지를 뽑아 `Error("pump panicked: ..")` 로 변환한다.
/// pump 클로저에서 분리해 둔 이유: 이 매핑이 load-bearing(panic→Failed 전이)이라 실제 PTY
/// child 없이 단위테스트로 직접 검증할 수 있게 한다.
fn resolve_pump_reason(result: std::thread::Result<TerminalReason>) -> TerminalReason {
    match result {
        Ok(reason) => reason,
        Err(payload) => {
            // panic payload 는 보통 &str 또는 String — 둘 다 시도.
            let msg = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "<non-string panic payload>".to_string());
            TerminalReason::Error(format!("pump panicked: {msg}"))
        }
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

        // ── 자연 종료 감지 watcher(콘솔 전용 — 이 detection 은 PtyTransport 안에만 둔다) ──
        // 문제: Windows ConPTY 는 master 가 살아있는 한 자식이 스스로 exit 해도 reader 에 EOF 를
        //   주지 않는다. 그래서 자연 종료(cmd /c exit) 시 pump 의 blocking read 가 영원히 안 깬다
        //   → core.finish 미호출 → reaper 신호 안 감. (kill 경로는 shutdown 이 master 를 drop 하므로
        //   EOF 가 와서 정상.) 이를 보완: 자식 종료를 폴링 감지해 **master 를 drop** 함으로써
        //   기존 EOF→pump break→finish→reaper 경로를 그대로 타게 한다.
        //
        // ★shutdown 플래그는 건드리지 않는다★ — set 하면 pump 가 Killed 로 전이한다. 자연 종료는
        //   Exited{code} 로 정확히 산출돼야 status·로그가 맞으므로(ADR-0083 이후 reaper disposition 은
        //   셧다운만 KeepAsIs·그 외 전부 KeepDisableAutoRestore 라 삭제엔 무영향이나, exit code 구분은
        //   status/진단에 여전히 필요) watcher 는 master drop 만 한다. reason 산출은 pump 가 try_wait 의
        //   exit code 로 한다(코드 무변경).
        //
        // ★데드락/이중 wait 안전★: WinChild::try_wait/wait/kill 은 내부 proc 핸들을 try_clone 후
        //   외부 Mutex 를 즉시 해제하므로, watcher 가 우리 child Mutex 를 **짧게만**(try_wait 1회)
        //   잡고 sleep 한다 → shutdown 의 child.lock()+kill+wait 와 경합해도 곧 풀려 데드락 없음.
        //   Windows 는 좀비 reaping 이 없고 핸들 보유 = 종료코드 보존이라, watcher 의 try_wait 와
        //   이후 pump 의 try_wait 가 같은 code 를 반복 회수해도 무해(이중 reap 문제 없음).
        // ★멱등★: master drop 은 watcher·shutdown 둘 다 take() → 먼저 take 한 쪽만 닫고 나머진 None.
        let watcher_child = self.child.clone();
        let watcher_master = self.master.clone();
        let watcher_shutdown = self.shutdown.clone();
        let watcher = std::thread::Builder::new()
            .name("engram-pty-watcher".into())
            .spawn(move || loop {
                // 1. shutdown(kill) 진행 중이면 watcher 역할은 끝 — kill 경로가 master 를 drop 해
                //    pump 를 깨운다. 자연 종료 감지가 불필요하므로 즉시 종료(자원 회수).
                if watcher_shutdown.load(Ordering::Relaxed) {
                    return;
                }
                // 2. 자식 종료 폴링(child Mutex 를 짧게만 보유 — try_wait 후 즉시 drop guard).
                let exited = {
                    let mut child = match watcher_child.lock() {
                        Ok(g) => g,
                        Err(poisoned) => poisoned.into_inner(),
                    };
                    matches!(child.try_wait(), Ok(Some(_)))
                };
                if exited {
                    // 3. 자식이 스스로 종료됨 → master drop 으로 pump read 를 EOF 로 깨운다.
                    //    shutdown 미set 이므로 pump 는 Exited{code} 로 finish 한다(자연 종료 분류).
                    let _ = watcher_master.lock().expect("master poisoned").take();
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            })
            .expect("spawn pty watcher thread");
        // watcher 핸들은 detach(join 하지 않음) — kill/자연종료 어느 쪽이든 곧 return 한다.
        // (shutdown set → 즉시 return / 자식 종료 → master drop 후 return.) 자원 자체 회수만 보장.
        drop(watcher);

        let handle = std::thread::spawn(move || {
            // ── B-2: pump 본체를 catch_unwind로 감싼다 ──
            // pump 스레드가 panic하면(emit/read/try_wait 어디서든) 그 agent 출력이 영구 silent
            // 정지하는데 감지·상태전이가 없었다(§5 위반). 본체를 catch_unwind로 잡아, panic이면
            // core.finish(Error)로 Failed 전이시켜 사용자/LLM에게 가시화한다.
            //
            // ★UnwindSafe★: 클로저가 잡는 reader/buf/child/shutdown은 panic 후 더 쓰지 않고
            //   버리므로(스레드가 곧 종료) 논리적 불변 깨짐이 없다 → AssertUnwindSafe로 명시.
            //   Mutex(child) 자체는 UnwindSafe지만 캡처 묶음(특히 dyn Read reader)이 아니므로 감싼다.
            // ★Mutex poison 범위(정확히)★: 아래 reason 산출의 child.lock()만 poison-tolerant
            //   (into_inner)하게 다룬다 — child는 이 transport(=이 agent) 전용이라 그 poison이
            //   다른 agent로 전파되지 않는다.
            //   단 panic이 pump_core.emit() 내부(replay/subscribers lock 보유 중)에서 터지면 그
            //   core Mutex들은 poison되고 poison-tolerant가 아니다 → 이후 그 agent에 새 구독/조회가
            //   오면 subscribe_from/status/snapshot의 .expect("...poisoned")가 재-panic한다. 그러나
            //   (a) core는 agent 전용이라 다른 agent로 전파 안 되고, (b) 그 재-panic은 연결 task
            //   (read_task) 안이라 tokio가 그 task만 격리(데몬·타 agent 무사)한다. 또 현재 emit
            //   경로에는 실제 panic 원이 없다(WsOutputSink::send는 panic 대신 Err 반환). 그래서
            //   core lock은 의도적으로 fail-fast(expect) 유지 — poison은 "데이터 불일치 가능"의
            //   신호라 무시(into_inner)보다 그 agent를 죽이는 게 안전하다.
            let normal_reason = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
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
                // ★poison-tolerant★: lock이 poison이어도 into_inner로 데이터를 꺼내 또 panic하지 않는다.
                let code = {
                    let mut child = match child.lock() {
                        Ok(g) => g,
                        Err(poisoned) => poisoned.into_inner(),
                    };
                    match child.try_wait() {
                        Ok(Some(status)) => Some(status.exit_code() as i32),
                        _ => None,
                    }
                };

                // shutdown store(Release)와 페어링되도록 Acquire로 읽는다(현 drain.rs와 동일).
                // 이미 terminal이면 덮지 않는 idempotent 규칙은 core.finish의 finalize 게이트가 담당.
                if shutdown.load(Ordering::Acquire) {
                    TerminalReason::Killed
                } else {
                    TerminalReason::Exited { code }
                }
            }));

            // panic이면 Error reason으로, 정상이면 산출된 reason으로 종료 전이.
            // ★finalize 1회 보존★: panic 경로의 finish와 (정상 EOF 직후 race로) 중복 호출돼도
            //   OutputCore.finalized.swap(AcqRel)가 정확히 1회만 통과시킨다. catch_unwind는 한
            //   클로저가 panic하면 그 안의 정상 finish는 도달 못 하므로, 여기서 정확히 한 번만
            //   finish가 불린다(panic→Error 또는 정상→reason). 이중 호출 자체가 발생하지 않는다.
            let reason = resolve_pump_reason(normal_reason);

            // terminal 알림 주체 = pump(=core.finish). finalize 정확히 1회.
            pump_core.finish(reason);

            // G-1: 완료 신호. core.join_pump의 recv_timeout가 받는다. 수신측이 이미
            // 사라졌어도(타임아웃 후 detach) 무시. ★panic 경로에서도 반드시 보낸다★ —
            // catch_unwind로 panic을 흡수했으므로 이 send에 도달한다(join_pump가 5s 안 멈춤).
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

    /// 콘솔 물리 채널 caps — terminal-bytes 단방향 출력, raw 입력, resize/interrupt 가능.
    /// session(resume)·model 은 여기서 안 만든다 — backend 소관(claude=resume, shell=no-resume).
    /// 최종 Capabilities 는 `Capabilities::compose(this, backend_caps)` 로 합성된다.
    fn capabilities(&self) -> TransportCaps {
        TransportCaps {
            input: InputCaps {
                raw: true,
                message: false,
                attachment: false,
            },
            output: OutputCaps {
                terminal_bytes: true,
                // PtyTransport 는 터미널 바이트 캐리어 — 구조화 아님(xterm 렌더). ADR-0030/0044.
                structured: false,
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::types::{AgentId, AgentInfo, AgentStatus, StatusSink};
    use std::sync::Mutex;

    /// status 변경을 순서대로 수집하는 mock(격리 검증용).
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

    // ── B-2: panic catch_unwind 결과가 Error reason 으로 매핑되는지 ──
    #[test]
    fn resolve_pump_reason_panic_becomes_error() {
        // panic 한 클로저 → Error("pump panicked: ..") + 원래 메시지 보존.
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> TerminalReason {
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

    // ── B-2: 정상 종료 reason 은 그대로 passthrough ──
    #[test]
    fn resolve_pump_reason_normal_passthrough() {
        let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> TerminalReason {
            TerminalReason::Exited { code: Some(0) }
        }));
        assert!(matches!(
            resolve_pump_reason(ok),
            TerminalReason::Exited { code: Some(0) }
        ));
        // Killed 도 보존.
        let killed =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> TerminalReason {
                TerminalReason::Killed
            }));
        assert!(matches!(
            resolve_pump_reason(killed),
            TerminalReason::Killed
        ));
    }

    // ── B-2: panic reason → core.finish → Failed 전이(정확히 1회) ──
    // pump 본체가 panic 했을 때 그 agent 의 OutputCore 가 Failed 로 전이하는지(가시화) +
    // finalize 1회 불변식이 깨지지 않는지(중복 finish 흡수)를 검증한다.
    #[test]
    fn panic_reason_finishes_core_as_failed_once() {
        let status_sink = CapturingStatusSink::new();
        let core = Arc::new(OutputCore::new(
            uuid::Uuid::new_v4(),
            0,
            status_sink.clone() as Arc<dyn StatusSink>,
        ));

        // pump 가 panic 했다고 가정 → resolve_pump_reason 으로 Error 산출 → finish.
        let panicked =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> TerminalReason {
                panic!("simulated pump panic")
            }));
        let reason = resolve_pump_reason(panicked);
        core.finish(reason);
        // (race 모사) 정상 EOF finish 가 뒤늦게 와도 finalize 1회로 무시.
        core.finish(TerminalReason::Exited { code: Some(0) });

        // status_sink 에 Failed 정확히 1회.
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

    // ── B-2: 한 agent 의 pump panic 이 다른 agent 로 전파되지 않음(격리) ──
    #[test]
    fn pump_panic_does_not_affect_other_agent() {
        let sink_a = CapturingStatusSink::new();
        let sink_b = CapturingStatusSink::new();
        let core_a = Arc::new(OutputCore::new(
            uuid::Uuid::new_v4(),
            0,
            sink_a.clone() as Arc<dyn StatusSink>,
        ));
        let core_b = Arc::new(OutputCore::new(
            uuid::Uuid::new_v4(),
            0,
            sink_b.clone() as Arc<dyn StatusSink>,
        ));

        // A 의 pump 가 panic → A 만 Failed.
        let panicked =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> TerminalReason {
                panic!("A panic")
            }));
        core_a.finish(resolve_pump_reason(panicked));

        // B 는 정상 출력 후 정상 종료 — A 의 panic 과 무관.
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
