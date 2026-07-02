//! StdioTransport — 파이프(stdin/stdout/stderr) 자식 프로세스용 `AgentTransport` 구현.
//!
//! PtyTransport(ConPTY 대화형)와 달리 **PTY 없는 평범한 파이프 프로세스**다. claude json 모드
//! (`-p --output-format stream-json`, 헤드리스)가 이 transport로 뜬다(ADR-0044). 터미널 모드는
//! 그대로 PtyTransport. 같은 AgentSession 조립에서 transport만 갈아끼운다.
//!
//! ★바보 파이프 불변(ADR-0044)★: pump는 stdout 바이트를 **해석하지 않고** 그대로
//!   `OutputEvent::TerminalBytes`로 core에 넘긴다(캐리어 variant 재사용 — 새 variant 금지).
//!   NDJSON 파싱은 프론트(RichSlot) 몫이다. transport/core/데몬은 내용을 모른다.
//!
//! ★PTY와 결정적 차이 — watcher 불필요★: ConPTY는 master가 살아 있으면 자식이 스스로 exit해도
//!   reader에 EOF를 안 줘서 PtyTransport가 자연 종료 감지용 watcher 스레드를 둔다. **파이프는
//!   자식(및 자식 트리)이 write 핸들을 모두 닫으면 read가 EOF(Ok(0))로 깬다** — 자연 종료든
//!   kill이든 동일하게 pump가 깨므로 별도 watcher가 없다(그만큼 단순).
//!
//! 소유권: child(Arc<Mutex> — pump의 try_wait와 shutdown의 kill+wait가 공유) · stdin/stdout/stderr
//!   (start에서 stdout/stderr를 take해 pump/drain 스레드로 move) · shutdown flag(Arc) · job(Windows).
//!
//! tauri import 0. unsafe 0(platform/windows.rs 제외).

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use crate::agent::output_core::OutputCore;
use crate::agent::transport::AgentTransport;
use crate::agent::types::{
    CommandSpec, ControlCaps, InputCaps, InputEvent, OutputCaps, OutputEvent, PtyError,
    TerminalReason, TransportCaps,
};
use crate::logging::mask_secrets;

#[cfg(windows)]
use crate::agent::platform::JobObjectHandle;

/// 파이프 자식 프로세스 transport. child + 세 파이프 + shutdown flag + structured caps + (Windows) Job.
///
/// stdin은 send_input(쓰기)·shutdown(kill 후 best-effort try_lock 정리)이 공유하므로 Mutex<Option<..>>.
/// stdout/stderr는 start()에서 take해 각각 pump·drain 스레드로 move한다(None이면 이미 시작됨 — 멱등 방어).
pub struct StdioTransport {
    /// pump(try_wait)와 shutdown(kill+wait)이 공유. std Child는 wait 후 exit status를 캐시하므로
    /// shutdown이 먼저 reap해도 pump의 try_wait가 같은 status를 회수한다(이중 wait 무해).
    child: Arc<Mutex<Child>>,
    /// 입력 파이프. send_input이 blocking write_all 내내 이 락을 쥔다 → shutdown 은 이 락을
    /// **blocking 으로 기다리면 안 된다**(데드락, FIX 1). shutdown 은 kill 후 try_lock 으로만
    /// best-effort 정리하고, 못 얻으면 transport drop 시 OS 회수에 맡긴다.
    stdin: Mutex<Option<ChildStdin>>,
    /// 출력 파이프. start()에서 take해 pump 스레드로 move. None이면 이미 시작됨.
    stdout: Mutex<Option<ChildStdout>>,
    /// 에러 파이프. start()에서 take해 drain 스레드로 move(라인별 debug! — claude 진행 noise, 출력 스트림엔 안 섞음).
    stderr: Mutex<Option<ChildStderr>>,
    /// shutdown(kill) 진행 신호. set(Release)면 pump가 종료 시 Killed로 전이(pump가 Acquire).
    shutdown: Arc<AtomicBool>,
    /// 이 파이프가 나르는 출력이 구조화 스트림(NDJSON)인지. ★조립점 주입(ADR-0044/0030)★:
    /// "구조화냐"는 파이프가 아니라 claude `--output-format`(backend/mode 지식)이 정하므로,
    /// select_transport 가 mode 로부터 주입한다(하드코딩 금지 — 평문 stdio 엔 false). capabilities()가 그대로 신고.
    structured: bool,
    #[cfg(windows)]
    job_handle: JobObjectHandle,
}

impl StdioTransport {
    /// CommandSpec으로 파이프 자식 spawn + (Windows) Job 편입 + 세 파이프 확보. **pump는 아직
    /// 안 띄운다**(start에서). child_pid를 함께 반환한다(claude 세션 추적 부착용 — 호출자 사용).
    ///
    /// PtyTransport::open과 시그니처를 맞추되 cols/rows가 없다(파이프엔 터미널 크기 개념 없음).
    /// `structured`: 이 파이프가 나르는 출력이 NDJSON(구조화)인지 — 호출자(select_transport)가
    /// mode 로부터 주입한다(파이프는 내용을 모름, ADR-0044/0030). capabilities().output.structured 로 신고.
    pub fn open(
        spec: &CommandSpec,
        structured: bool,
    ) -> Result<(StdioTransport, Option<u32>), PtyError> {
        // 1. Command 구성. transport는 claude/codex를 모른다 — backend가 산출한 spec만 본다.
        //    Windows shim(claude.cmd) 처리는 backend/console_command가 이미 `cmd.exe /c claude …`로
        //    감싼 spec을 준다(PtyTransport와 동일 경로) — 여기선 그 program/args를 그대로 실행한다.
        let mut cmd = Command::new(&spec.program);
        cmd.args(&spec.args);
        cmd.current_dir(&spec.cwd);
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        // 세 파이프 모두 확보 — stdout/stderr를 우리가 읽어야 자식이 파이프 버퍼 full로 블록되지 않는다.
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Windows: 헤드리스 백그라운드 프로세스라 콘솔 창이 튀지 않게 CREATE_NO_WINDOW.
        //   (데몬은 창 없는 프로세스일 수 있어 cmd.exe shim이 콘솔을 새로 띄우는 깜빡임을 막는다.)
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| PtyError::SpawnFailed(format!("stdio spawn: {e}")))?;

        let child_pid = Some(child.id());

        // 2. 세 파이프 take(Command가 piped로 열어 Child에 담아둔 것).
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        // 3. Windows: Job 생성 + child 편입(트리 kill — 손자 claude까지). PtyTransport와 동일 순서.
        #[cfg(windows)]
        let job_handle = {
            let job = JobObjectHandle::new()?;
            if let Some(pid) = child_pid {
                job.assign(pid)?;
            }
            job
        };

        let transport = StdioTransport {
            child: Arc::new(Mutex::new(child)),
            stdin: Mutex::new(stdin),
            stdout: Mutex::new(stdout),
            stderr: Mutex::new(stderr),
            shutdown: Arc::new(AtomicBool::new(false)),
            structured,
            #[cfg(windows)]
            job_handle,
        };

        Ok((transport, child_pid))
    }
}

/// catch_unwind 결과 → 종료 reason 매핑. pty.rs resolve_pump_reason의 파이프판(동일 규칙):
/// 정상이면 산출 reason 그대로, panic이면 payload에서 메시지를 뽑아 Error로. pump 스레드가
/// 어디서든 panic하면 그 agent가 영구 silent 정지하므로 Failed로 가시화한다(§5).
fn resolve_pump_reason(result: std::thread::Result<TerminalReason>) -> TerminalReason {
    match result {
        Ok(reason) => reason,
        Err(payload) => {
            let msg = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "<non-string panic payload>".to_string());
            TerminalReason::Error(format!("pump panicked: {msg}"))
        }
    }
}

impl AgentTransport for StdioTransport {
    /// pump 스레드(stdout→core) + stderr drain 스레드 기동 + core 연결.
    /// stdout이 이미 take됐으면(재호출) 아무것도 안 한다(멱등 방어 — pty와 동형).
    fn start(&self, core: Arc<OutputCore>) {
        // 로그 계측용 agent 식별자(스레드로 move해 필터 키로 씀). core 가 보유한 불변값.
        let agent_id = core.id();

        let stdout = match self.stdout.lock().expect("stdout poisoned").take() {
            Some(s) => s,
            None => return,
        };

        // ── stderr drain 스레드 ──
        // ★왜 drain 하나(파이프 fill 방지)★: stderr 파이프를 안 비우면 자식이 stderr 버퍼 full 로
        //   블록해 진행이 멈춘다. 그래서 반드시 한 줄씩 읽어 흘린다(bounded — 무한 버퍼링 없음).
        // ★왜 출력 스트림에 안 섞나(ADR-0044)★: json 모드 stdout은 NDJSON이라 프론트 RichSlot이
        //   라인 단위로 파싱한다. stderr(경고·진단 텍스트)를 같은 스트림에 병합하면 NDJSON 중간에
        //   비-JSON 라인이 껴 파서가 깨진다. 그래서 stderr는 출력과 분리해 라인별 로그로만 흘린다.
        // ★레벨=debug(FIX 4/logging-conventions)★: claude 는 진행·진단 텍스트를 stderr 로 흘리는 게
        //   정상 noise다 — warn 으로 찍으면 레벨 규약(warn=비정상)을 위반하고 로그를 범람시킨다.
        if let Some(stderr) = self.stderr.lock().expect("stderr poisoned").take() {
            let spawn_result = std::thread::Builder::new()
                .name("engram-stdio-stderr".into())
                .spawn(move || {
                    let reader = BufReader::new(stderr);
                    for line in reader.lines() {
                        match line {
                            // ★mask_secrets(FIX 4)★: 외부 프로세스(claude) 출력이라 자격증명이 섞일 수
                            //   있다 — 신선한 external-output 로그 경로는 호출자가 명시 마스킹(logging §보안).
                            Ok(l) if !l.is_empty() => {
                                tracing::debug!(target: "agent_stderr", agent = %agent_id, "{}", mask_secrets(&l))
                            }
                            Ok(_) => {}
                            Err(_) => break,
                        }
                    }
                });
            // ★spawn 실패를 삼키지 않는다(FIX 4/logging 계측 의무)★: drain 스레드가 안 뜨면 stderr
            //   파이프가 안 비워져 자식이 블록될 수 있다 — 조용히 버리지 말고 agent 맥락과 함께 warn.
            if let Err(e) = spawn_result {
                tracing::warn!(agent = %agent_id, "stdio stderr drain 스레드 기동 실패: {e}");
            }
        }

        // ── pump 스레드(stdout→core) ──
        let (done_tx, done_rx) = mpsc::channel();
        let pump_core = core.clone();
        let child = self.child.clone();
        let shutdown = self.shutdown.clone();

        let handle = std::thread::spawn(move || {
            // pty.rs와 동일하게 pump 본체를 catch_unwind로 감싼다(panic→Failed 가시화).
            // ★UnwindSafe★: 잡은 stdout/buf/child/shutdown은 panic 후 버려지므로(스레드 종료)
            //   논리 불변 깨짐 없음 → AssertUnwindSafe.
            let normal_reason = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut reader = stdout;
                let mut buf = [0u8; 4096];

                loop {
                    // 1. blocking read. 자식 트리가 stdout write 핸들을 모두 닫으면(자연 종료 또는
                    //    kill+Job terminate) Ok(0)=EOF로 깬다. Err도 종료로 간주.
                    let n = match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };

                    // 2. shutdown 보조 확인(pty와 동일 안전망) — read가 데이터를 막 반환한 직후
                    //    kill이 걸린 경우. 보통은 write 핸들 close로 인한 EOF가 먼저 깨운다.
                    if shutdown.load(Ordering::Relaxed) {
                        break;
                    }

                    // 3. ★바보 파이프★: 바이트를 해석하지 않고 그대로 emit(seq/replay/fanout은 core).
                    pump_core.emit(OutputEvent::TerminalBytes(buf[..n].to_vec()));
                }

                // exit code 취득(pty와 동일 규율 — poison-tolerant into_inner).
                // 주: kill 경로는 shutdown의 kill()+wait()가 이미 reap했을 수 있으나 std Child는
                //     status를 캐시하므로 try_wait가 Some을 돌려준다. 그래도 shutdown=true면 아래서
                //     code 미사용(Killed)이라 무해.
                let code = {
                    let mut child = match child.lock() {
                        Ok(g) => g,
                        Err(poisoned) => poisoned.into_inner(),
                    };
                    match child.try_wait() {
                        Ok(Some(status)) => status.code(),
                        _ => None,
                    }
                };

                // shutdown store(Release)와 페어링되게 Acquire로 읽는다.
                if shutdown.load(Ordering::Acquire) {
                    TerminalReason::Killed
                } else {
                    TerminalReason::Exited { code }
                }
            }));

            let reason = resolve_pump_reason(normal_reason);

            // terminal 알림 주체 = pump(=core.finish). finalize 정확히 1회(core가 게이트).
            pump_core.finish(reason);

            // G-1: 완료 신호(core.join_pump의 recv_timeout가 받는다). 수신측이 사라졌어도 무시.
            let _ = done_tx.send(());
        });

        core.attach_pump(handle, done_rx);
    }

    /// 입력 전달 — Raw 바이트를 자식 stdin으로 쓴다. json 모드에선 이 바이트가 이미 backend가
    /// 감싼 stream-json 유저 턴 라인(`{"type":"user",…}\n`)이다 — transport는 그 형태를 모른다
    /// (AgentSession이 InputEncoder로 감싸 Raw로 넘긴다, ADR-0044 격리). 여기선 그냥 쓴다.
    fn send_input(&self, input: InputEvent) -> Result<(), PtyError> {
        match input {
            InputEvent::Raw(bytes) => {
                let mut guard = self.stdin.lock().expect("stdin poisoned");
                let stdin = guard
                    .as_mut()
                    .ok_or_else(|| PtyError::WriteFailed("stdin closed".into()))?;
                stdin
                    .write_all(&bytes)
                    .map_err(|e| PtyError::WriteFailed(e.to_string()))?;
                stdin
                    .flush()
                    .map_err(|e| PtyError::WriteFailed(e.to_string()))?;
                Ok(())
            }
        }
    }

    /// 미지원 — 파이프엔 터미널 크기 개념이 없다(caps.resize=false). unsupported-op 패턴(ApiTransport 동형).
    fn resize(&self, _cols: u16, _rows: u16) -> Result<(), PtyError> {
        Err(PtyError::Unsupported(
            "StdioTransport::resize (파이프는 터미널 크기 없음)".into(),
        ))
    }

    /// 미지원 — 파이프엔 PTY Ctrl-C 주입 경로가 없다. ★ADR-0044 MVP 한계(의도된 미구현)★:
    /// 진행 중 작업 중단(interrupt)은 후속 스파이크. kill(shutdown)만 가능. caps.interrupt=false.
    fn interrupt(&self) -> Result<(), PtyError> {
        Err(PtyError::Unsupported(
            "StdioTransport::interrupt (ADR-0044 MVP 미지원 — 파이프 Ctrl-C 없음, 후속 스파이크)"
                .into(),
        ))
    }

    /// 자원 강제 종료(멱등) — ADR-0001 2동사의 파이프판. pump 종료 대기는 여기서 안 함(join_pump 몫).
    ///
    /// ★kill 인과(파이프판)★: PtyTransport는 master drop→ConPTY close→reader EOF로 pump를 깨우지만,
    ///   파이프는 **자식 트리가 stdout write 핸들을 모두 닫아야** reader가 EOF로 깬다. 그래서
    ///   child.kill + Job terminate(손자 claude까지)로 트리를 통째 죽여 write 핸들을 닫는 것이
    ///   인과의 핵심이다(cmd.exe만 죽이고 claude가 살아 있으면 write 핸들이 안 닫혀 EOF가 안 온다).
    ///
    /// ★순서 불변 — stdin close 는 kill 보다 절대 먼저 오면 안 된다(데드락, FIX 1)★:
    ///   send_input 은 stdin Mutex 를 **blocking write_all 내내** 쥔다. 자식이 stdin 을 안 읽으면
    ///   (파이프 backpressure) 그 write_all 이 영원히 블록해 락을 놓지 않는다. 이때 kill **전에**
    ///   `stdin.lock()` 으로 닫으려 하면 그 락을 영영 못 얻어 kill 에 도달조차 못 하고 → pump 가
    ///   깨지 못해 → core.join_pump 가 영구 hang 한다(ADR-0001 인과가 멈춤). 그래서 **kill + Job
    ///   terminate 를 먼저** 한다: 자식을 죽이면 파이프가 깨져 블록된 write_all 이 에러로 풀리고
    ///   락이 해제된다. 그 뒤에야 try_lock 으로 stdin 을 best-effort 정리한다(blocking lock 절대 금지).
    /// ※graceful-exit-via-stdin-close 는 필요 없다 — 어차피 여기서 kill 하므로. (예전 "stdin EOF →
    ///   graceful" 기대는 제거: kill 경로에선 graceful 종료를 기다리지 않는다.)
    fn shutdown(&self) {
        // 1. shutdown 신호 — pump가 종료 시 Killed로 전이(store Release, pump가 Acquire).
        self.shutdown.store(true, Ordering::Release);

        // 2. child kill + wait(reap, 좀비 방지). ★stdin 을 만지기 전에 먼저★(위 순서 불변 — 데드락 회피).
        //    두 번째 호출은 이미 죽어 Err — 무시(멱등).
        {
            let mut child = self.child.lock().expect("child poisoned");
            let _ = child.kill();
            let _ = child.wait();
        }

        // 3. Windows: Job 전체 종료 → 손자(cmd 아래 claude)까지. 이게 claude의 stdout write 핸들을
        //    닫아 pump reader를 EOF로 깨운다(인과의 핵심). 비Windows는 child.kill이 직접 자식
        //    (claude, shim 없음)을 죽여 write 핸들이 닫힌다.
        #[cfg(windows)]
        {
            let _ = self.job_handle.terminate(1);
        }

        // 4. stdin best-effort 정리 — 위 kill 이 파이프를 깨 blocked write_all 이 풀리며 send_input 이
        //    락을 놓으므로 try_lock 이 대개 성공한다. 못 얻으면(아직 write_all 이 안 풀린 찰나) 그냥
        //    skip: 데드락 회피를 위해 **blocking lock 을 절대 걸지 않는다**. 미정리 ChildStdin 은
        //    transport drop 시 OS 가 회수하므로 누수 없음(kill 로 이미 파이프는 끊겼다).
        if let Ok(mut guard) = self.stdin.try_lock() {
            let _ = guard.take();
        }
    }

    /// 파이프 물리 채널 caps — raw 입력, resize/interrupt 불가, terminal_bytes=false(터미널 아님).
    /// ★output.structured 는 주입값(ADR-0030/0044)★: "이 바이트가 NDJSON 인가"는 파이프가 아니라
    ///   claude `--output-format`(backend/mode 지식)이 정한다 — 파이프는 내용을 모른다(통로 무정제).
    ///   그래서 하드코딩하지 않고 select_transport 가 mode 로부터 주입한 self.structured 를 그대로 신고한다
    ///   (예전 하드코딩 true 는 평문 stdio 에도 거짓말을 했다). output 이 transport 소유 영역이라는
    ///   출처 분리(ADR-0030)는 그대로 — 값만 조립점에서 주입받는다.
    /// ※structured 는 "이 스트림은 터미널이 아니다"라는 렌더 힌트일 뿐 내용 해석 아님. caps 기반
    ///   렌더러 분기(xterm vs RichSlot)는 **M2 예정이며 아직 미배선**이다(M0 스파이크는 viewStore.richSlots
    ///   오버레이로 분기) — 이 필드를 "현재 렌더 분기의 유일 근거"로 오독하지 말 것(FIX 6c).
    /// session(resume)·model은 backend 소관이라 여기서 안 만든다(TransportCaps엔 그 필드 없음).
    fn capabilities(&self) -> TransportCaps {
        TransportCaps {
            input: InputCaps {
                // stdin에 raw 바이트를 쓴다(그 바이트가 json 라인인지는 backend/session이 결정).
                raw: true,
                message: false,
                attachment: false,
            },
            output: OutputCaps {
                // 터미널 바이트 아님(파이프). structured 는 조립점 주입값(json 모드=true, 평문 stdio=false).
                terminal_bytes: false,
                structured: self.structured,
                markdown: false,
                tool_events: false,
                usage: false,
            },
            control: ControlCaps {
                resize: false,
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

    // ── resolve_pump_reason: panic → Error 매핑(pty.rs와 동일 규칙) ──
    #[test]
    fn resolve_pump_reason_panic_becomes_error() {
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> TerminalReason {
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
        let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> TerminalReason {
            TerminalReason::Exited { code: Some(0) }
        }));
        assert!(matches!(
            resolve_pump_reason(ok),
            TerminalReason::Exited { code: Some(0) }
        ));
    }

    // ── caps 정직성(FIX 2): structured 는 주입값 그대로 신고(하드코딩 아님) + resize/interrupt false ──
    // 실 프로세스 없이 capabilities()만 검증하려면 인스턴스가 필요하다 → open으로 harmless 자식
    // (echo, 즉시 종료)을 띄운 뒤 caps만 확인하고 shutdown으로 정리한다.
    #[cfg(windows)]
    #[test]
    fn capabilities_structured_reflects_injected_value_and_no_resize_interrupt() {
        let spec = CommandSpec {
            program: "cmd.exe".into(),
            args: vec!["/c".into(), "echo caps-probe".into()],
            env: vec![],
            cwd: std::path::PathBuf::from("."),
        };

        // 평문 stdio(구조화 아님) 주입 → structured=false 로 정직 신고(예전 하드코딩 true 회귀 방지).
        let (plain, _pid) = StdioTransport::open(&spec, false).expect("open plain");
        assert!(
            !plain.capabilities().output.structured,
            "평문 stdio 주입 → structured=false(파이프가 아니라 mode 가 결정)"
        );
        plain.shutdown();

        // json 캐리어(구조화) 주입 → structured=true + 파이프 물리 caps 나머지 검증.
        let (json, _pid) = StdioTransport::open(&spec, true).expect("open json");
        let caps = json.capabilities();
        assert!(caps.output.structured, "json 캐리어 주입 → structured=true");
        assert!(!caps.output.terminal_bytes, "터미널 바이트 아님");
        assert!(!caps.control.resize, "파이프 resize 불가");
        assert!(!caps.control.interrupt, "MVP interrupt 미지원");
        assert!(caps.input.raw, "stdin raw 쓰기 가능");
        // interrupt/resize는 Unsupported를 반환(unsupported-op 패턴).
        assert!(matches!(json.interrupt(), Err(PtyError::Unsupported(_))));
        assert!(matches!(json.resize(80, 24), Err(PtyError::Unsupported(_))));
        json.shutdown();
    }

    // ── FIX 1 회귀: send_input 이 stdin 락을 쥔 채 블록해도 shutdown 이 데드락 없이 완료된다 ──
    // 재현: 자식이 stdin 을 절대 읽지 않게 하고(ping sleep), 큰 페이로드를 write_all → 파이프
    //   backpressure 로 write_all 이 락을 쥔 채 블록. 그 상태에서 shutdown 을 호출한다. 버그(=stdin
    //   close 를 kill 보다 먼저)면 shutdown 이 그 락을 영영 못 얻어 hang → 이 테스트가 타임아웃으로
    //   잡는다. 픽스(kill 먼저 → try_lock)면 shutdown 이 즉시 완료된다.
    #[cfg(windows)]
    #[test]
    fn shutdown_completes_even_if_send_input_blocks_on_full_pipe() {
        use std::time::{Duration, Instant};

        // ping -n 30 = ~30s 동안 살아있으며 stdin 을 읽지 않는다(간편한 sleep). 소량 stdout 은
        // 파이프 버퍼 아래라 자식이 stdout 으로도 블록하지 않는다 → stdin 미소비 상태 유지.
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
        let (transport, _pid) = StdioTransport::open(&spec, true).expect("open");
        let transport = Arc::new(transport);

        // writer: 파이프 버퍼를 훨씬 초과하는 8MB 를 write → 자식이 안 읽으니 write_all 이 락 쥔 채 블록.
        let writer = transport.clone();
        let writer_thread = std::thread::spawn(move || {
            let big = vec![b'x'; 8 * 1024 * 1024];
            let _ = writer.send_input(InputEvent::Raw(big));
        });

        // writer 가 write_all 에 진입해 stdin 락을 확실히 잡도록 잠깐 양보(넉넉히).
        std::thread::sleep(Duration::from_millis(500));

        // shutdown 을 별도 스레드에서 돌리고 경과시간으로 완료를 단언한다(픽스면 즉시, 버그면 hang).
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
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "shutdown deadlock 회귀(FIX 1)"
        );

        // kill 로 파이프가 끊겨 blocked write_all 이 에러로 풀리고 writer 가 종료된다.
        let _ = writer_thread.join();
    }
}
