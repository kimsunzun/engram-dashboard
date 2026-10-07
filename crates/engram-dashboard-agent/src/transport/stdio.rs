//! StdioTransport — 파이프(stdin/stdout/stderr) 자식 프로세스용 `AgentTransport` 구현.
//!
//! PtyTransport(ConPTY 대화형)와 달리 **PTY 없는 평범한 파이프 프로세스**다. claude json 모드
//! (`-p --output-format stream-json`, 헤드리스)가 이 transport로 뜬다(ADR-0044). 터미널 모드는
//! 그대로 PtyTransport. 같은 AgentSession 조립에서 transport만 갈아끼운다.
//!
//! ★무정제 불변(ADR-0044/0045)★: transport 층은 stdout 바이트의 스키마를 모른다 — decoder 가
//!   없으면 `OutputEvent::TerminalBytes`로 그대로 넘기고(캐리어 variant 재사용), 있으면 주입된
//!   decoder 를 적용만 한다. 파싱은 backend decoder 소관이지 이 층도 프론트도 아니다.
//!   끊기 줄과 그 줄이 나간 뒤 부를 것([`InterruptOut`])도 입력 큐로 넘기기만 하고, 무리 손잡이와 물러남 표시
//!   (`StdioTransport::process_group`)는 내주기만 한다 — 쓰임은 backend 가 안다.
//!
//! ★PTY와 결정적 차이 — watcher 불필요★: ConPTY는 master가 살아 있으면 자식이 스스로 exit해도
//!   reader에 EOF를 안 줘서 PtyTransport가 자연 종료 감지용 watcher 스레드를 둔다. **파이프는
//!   자식(및 자식 트리)이 write 핸들을 모두 닫으면 read가 EOF(Ok(0))로 깬다** — 자연 종료든
//!   kill이든 동일하게 pump가 깨므로 별도 watcher가 없다(그만큼 단순).
//!
//! tauri import 0. unsafe 0.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use engram_dashboard_base::logging::mask_secrets;
use engram_dashboard_base::sync;
use engram_dashboard_platform::group::GroupOwner;
use engram_dashboard_platform::spawn::hide_console_window;

use crate::output_core::OutputCore;
use crate::transport::input_queue::{self, InputQueue, OnWritten};
use crate::transport::process_group::{ProcessGroup, RetiringSignal};
use crate::transport::{AgentTransport, OutputDecoder};
use crate::types::{
    CommandSpec, ControlCaps, InputCaps, InputEvent, OutputCaps, OutputEvent, PtyError,
    TerminalReason, TransportCaps,
};

/// 끊기 줄 함수가 주는 것 — stdin 에 쓸 줄 한 벌과, 그 줄이 실제로 파이프로 나간 뒤 라이터가 부를 것(계약 =
/// [`OnWritten`] — 나가지 못하면 불리지 않고 버려진다). 통로는 둘 다 입력 큐로 넘기기만 한다.
pub struct InterruptOut {
    pub bytes: Vec<u8>,
    pub on_written: Option<OnWritten>,
}

/// 「지금 도는 턴을 멈춰 달라」는 stdin 줄 한 벌을 만드는 backend 함수. 통로는 그 바이트의 뜻을 모른다(바보 파이프).
///
/// `None` = 「지금은 끊을 턴이 없다」 — 판정은 backend 가 하고 통로는 그대로 `Unsupported` 로 옮긴다. 파이프엔
/// Ctrl-C 같은 통로 자신의 끊기 수단이 없어서, 끊기는 이 줄을 만드는 쪽의 지식이다.
// ADR-0238
pub type InterruptLine = Arc<dyn Fn() -> Option<InterruptOut> + Send + Sync>;

pub struct StdioTransport {
    /// pump(try_wait)와 shutdown(kill+wait)이 공유. std Child는 wait 후 exit status를 캐시하므로
    /// shutdown이 먼저 reap해도 pump의 try_wait가 같은 status를 회수한다(이중 wait 무해).
    child: Arc<Mutex<Child>>,
    /// ★라이터 스레드와 `shutdown()` 이 **공유**한다 — PTY 쪽이 소유로 가는 것과 갈린다★. 이 핸들을
    ///   라이터에게 넘겨 버리면 `shutdown()` 4 단계의 `try_lock` 정리가 닿을 데가 없어진다. 그래서
    ///   공유하되, ★블로킹 `write_all` 로 이 락을 쥐는 것은 이제 라이터 스레드 하나뿐★이고 `shutdown()`
    ///   은 여전히 `try_lock` 만 쓴다(그 함수의 순서 불변식이 그대로 유효한 이유).
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    /// 아직 못 나간 입력. `send_input` 은 여기 넣고 **즉시** 돌아온다(모듈 = `transport::input_queue`).
    input: Arc<InputQueue>,
    /// start()에서 take해 pump 스레드로 move. None이면 이미 시작됨.
    stdout: Mutex<Option<ChildStdout>>,
    /// start()에서 take해 drain 스레드로 move.
    stderr: Mutex<Option<ChildStderr>>,
    /// shutdown(kill) 진행 신호. set(Release)면 pump가 종료 시 Killed로 전이(pump가 Acquire).
    shutdown: Arc<AtomicBool>,
    /// 물러남 표시 — [`AgentTransport::begin_retire`] 와 `shutdown()` 첫 줄이 세우고 아무도 내리지 않는다. 이 통로는
    /// 읽지 않고 [`Self::process_group`] 에 읽기 전용 사본으로 실어 내줄 뿐이다 — 쓰임은 통로가 모른다. `Arc` 인 것은
    /// 내준 사본이 통로보다 오래 살 수 있어서다(칸을 나눠 쥔다).
    retiring: Arc<AtomicBool>,
    /// 이 파이프가 나르는 출력이 구조화 스트림(NDJSON)인지. ★주입값이다(ADR-0044/0030/0191)★:
    /// "구조화냐"는 파이프가 아니라 그 프로그램의 출력 형식(backend 지식)이 정하므로, 이 통로를 만드는
    /// backend 가 `open` 인자로 주입한다(하드코딩 금지 — 평문 stdio 엔 false). capabilities()가 그대로 신고.
    structured: bool,
    /// 출력 정제 decoder(ADR-0004/0044). 이 통로를 만드는 backend 가 구조화 모드에 주입한다(없으면
    /// 바이트 직통 = 평문·터미널 경로). start()에서 take 해 pump 스레드로 move(=None 이면 이미 시작됨).
    decoder: Mutex<Option<Box<dyn OutputDecoder>>>,
    /// 끊기 줄 함수. `None` = 이 통로를 만든 backend 가 끊는 법을 주지 않았다 — `interrupt()` 는 늘 `Unsupported` 이고
    /// 능력도 거짓이다.
    // ADR-0238
    interrupt: Option<InterruptLine>,
    /// 이 통로가 띄운 프로세스 무리의 주인 — 하나뿐이고 밖으로는 [`Self::process_group`] 의 약한 손잡이만 내주므로,
    /// 통로가 사라지면 무리도 닫힌다(`KILL_ON_JOB_CLOSE`). ★마지막 칸으로 둔다★ — 칸은 선언 순서로 버려지므로 그
    /// 닫히는 때가 이 자리로 정해진다. 앞당겨도 되는지는 재 보지 않았고, 이 배치를 지키는 시험은 없다.
    group: GroupOwner,
}

impl StdioTransport {
    /// **pump는 아직 안 띄운다**(start에서). child_pid를 함께 반환한다(claude 세션 추적 부착용
    /// — 호출자 사용). PtyTransport::open과 시그니처를 맞추되 cols/rows가 없다.
    ///
    /// ★`decoder` 를 start 인자가 아니라 open 인자로 받는 이유★: decoder 주입은 StdioTransport
    /// 전용이라 공용 `AgentTransport::start` 시그니처를 건드리지 않는다(건드리면 Pty/Api 도
    /// 무의미한 None 인자를 강제로 받아야 함 — 파급 최소화).
    pub fn open(
        spec: &CommandSpec,
        structured: bool,
        decoder: Option<Box<dyn OutputDecoder>>,
    ) -> Result<(StdioTransport, Option<u32>), PtyError> {
        // Windows shim(claude.cmd) 처리는 backend 가 이미 platform `console_command` 로 감싼 spec
        //   (`cmd.exe /c claude …`)을 준다(PtyTransport와 동일 경로) — 여기선 그 program/args를 그대로 실행한다.
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

        // 헤드리스 백그라운드 프로세스다 — 데몬은 창 없는 프로세스일 수 있어 cmd.exe shim 이 콘솔을 새로 띄운다.
        hide_console_window(&mut cmd);

        let mut child = cmd
            .spawn()
            .map_err(|e| PtyError::SpawnFailed(format!("stdio spawn: {e}")))?;

        let child_pid = Some(child.id());

        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let group = GroupOwner::new()?;
        if let Some(pid) = child_pid {
            group.adopt(pid)?;
        }

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

    /// 이 통로가 띄운 프로세스 무리의 약한 손잡이 — 물러남 표시의 읽기 전용 사본을 함께 싣는다. 통로는 그것이
    /// 무엇에 쓰이는지 모른다(ADR-0044 「바보 파이프」). `None` = 이 OS 에서는 무리를 묶는 수단이 없다(Windows 밖).
    // ADR-0262
    pub(crate) fn process_group(&self) -> Option<ProcessGroup> {
        self.group
            .downgrade()
            .map(|group| ProcessGroup::new(group, RetiringSignal::of(&self.retiring)))
    }

    /// 끊기 줄 함수를 꽂는다 — 꽂으면 능력 `control.interrupt` 가 참이 된다(「지금 턴이 있다」가 아니라 「끊을 수 있는
    /// 통로다」). `open` 인자가 아닌 것은 주입 없는 호출자(평문 stdio · 시험)가 오늘 그대로 서게 하려는 것이다.
    // ADR-0238
    pub fn with_interrupt(mut self, line: InterruptLine) -> Self {
        self.interrupt = Some(line);
        self
    }
}

/// 라이터 스레드의 실제 쓰기 한 번. ★`shutdown()` 이 핸들을 이미 거뒀으면 `BrokenPipe` 로 떨어져
/// [`input_queue::drain`] 이 큐를 닫는다★ — 그 조합이 「kill 뒤에 남은 입력이 조용히 쌓이는」 갈래를 막는다.
fn write_stdin(stdin: &Mutex<Option<ChildStdin>>, bytes: &[u8]) -> std::io::Result<()> {
    // ★블로킹 `write_all` 을 이 락 아래서 하는 유일한 자리다★ — `shutdown()` 이 `try_lock` 을 쓰는
    //   근거(그 함수의 순서 불변식)가 여기를 가리킨다.
    let mut guard = sync::lock(stdin);
    let stdin = guard.as_mut().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::BrokenPipe, "stdin 이 이미 닫혔다")
    })?;
    stdin.write_all(bytes)?;
    stdin.flush()
}

/// pump 가 **어떤 길로 끝나든** 입력 큐를 닫아 라이터 스레드를 거둔다(사유·근거 = `pty.rs` 의 같은 이름).
struct WriterStop(Arc<InputQueue>);

impl Drop for WriterStop {
    fn drop(&mut self) {
        self.0.close("stdio 스트림이 끝났다 — 더 보낼 곳이 없다");
    }
}

/// pump 스레드가 어디서든 panic하면 그 agent가 영구 silent 정지하므로 Failed로 가시화한다(§5).
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
    /// stdout이 이미 take됐으면(재호출) 아무것도 안 한다(멱등 방어).
    fn start(&self, core: Arc<OutputCore>) {
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
        // ★drain 하면서 core 의 **진단 버퍼**에도 쌓는다(출력 링이 아니다 — ADR-0172)★: 위 ADR-0044
        //   근거대로 이 텍스트를 출력 스트림에 섞을 수는 없지만, 구조화 세션에서는 이것이 활성화 실패의
        //   **유일한 증거**다(claude 의 "No conversation found with session ID: …" 가 여기로만 온다).
        //   그래서 링과 분리된 작은 bounded 버퍼로 따로 붙든다 — 계약은 `OutputCore::push_diagnostic`.
        // ★두 번째 캡처를 만들지 않았다★: 이 drain 이 이미 stderr 를 라인 단위로 읽는 유일한 지점이라,
        //   여기 한 줄을 얹는 것으로 끝난다(파이프를 두 번 읽을 수는 없다).
        // ADR-0172
        if let Some(stderr) = self.stderr.lock().expect("stderr poisoned").take() {
            let diag_core = core.clone();
            let spawn_result = std::thread::Builder::new()
                .name("engram-stdio-stderr".into())
                .spawn(move || {
                    let reader = BufReader::new(stderr);
                    for line in reader.lines() {
                        match line {
                            // ★mask_secrets(FIX 4)★: 외부 프로세스(claude) 출력이라 자격증명이 섞일 수
                            //   있다 — 신선한 external-output 로그 경로는 호출자가 명시 마스킹(logging §보안).
                            //   ★마스킹은 버퍼에도 그대로 적용된다★: 이 텍스트는 실패 사유로 다시 로그에
                            //   실릴 수 있으므로, 원문을 붙들면 마스킹을 한 번 우회하는 경로가 생긴다.
                            Ok(l) if !l.is_empty() => {
                                let masked = mask_secrets(&l);
                                // ★쌓기가 로그보다 **먼저**다★: 기본 로그 필터는 warn 이라 아래 debug 줄은
                                //   평소 버려진다 — 분류를 그 줄에 매달면 로그 레벨이 기능을 켜고 끈다.
                                diag_core.push_diagnostic(&masked);
                                tracing::debug!(target: "agent_stderr", agent = %agent_id, "{}", masked)
                            }
                            Ok(_) => {}
                            Err(_) => break,
                        }
                    }
                });
            // ★spawn 실패를 삼키지 않는다(FIX 4/logging 계측 의무)★: 조용히 버리지 말고 agent
            //   맥락과 함께 warn.
            if let Err(e) = spawn_result {
                tracing::warn!(agent = %agent_id, "stdio stderr drain 스레드 기동 실패: {e}");
            }
        }

        // ── 입력 라이터 스레드 ──
        // ★아무도 join 하지 않는다 · kill 인과를 지연시킬 수 없다★ — 사유의 정본은 `pty.rs` 의 같은 자리.
        //   여기 stdio 판에만 있는 사실 하나: 이 스레드가 `write_stdin` 안에서 stdin 락을 쥔 채 매달릴 수
        //   있고, `shutdown()` 이 **kill 을 먼저** 하는 순서가 그것을 에러로 푼다(그 함수의 순서 불변식).
        // ★`stdout` 을 못 take 한 재호출 갈래에서는 여기 도달하지 않는다★ — 위에서 이미 return 했다.
        //   즉 라이터도 딱 한 번만 뜬다.
        {
            let queue = self.input.clone();
            let stdin = self.stdin.clone();
            let spawn_result = std::thread::Builder::new()
                .name("engram-stdio-writer".into())
                .spawn(move || {
                    // ★귀속(`agent = …`)이 없으면 이 경고가 흔적이 못 된다★ — 사유의 정본은
                    //   `input_queue::drain` doc. 아래 spawn 실패 갈래와 같은 필드다.
                    input_queue::drain(&queue, "stdio", agent_id, |bytes| {
                        write_stdin(&stdin, bytes)
                    });
                });
            if let Err(e) = spawn_result {
                // 라이터가 없으면 `send_input` 이 `Ok` 를 돌려주면서 바이트는 영영 안 나간다 — 조용한
                //   유실이라, 큐를 닫아 그 순간부터 정직하게 거절한다.
                let reason = format!("stdio 입력 라이터 스레드 기동 실패: {e}");
                tracing::warn!(agent = %agent_id, "{reason}");
                self.input.close(&reason);
            }
        }

        // ── pump 스레드(stdout→core) ──
        let (done_tx, done_rx) = mpsc::channel();
        let pump_core = core.clone();
        let child = self.child.clone();
        let shutdown = self.shutdown.clone();
        // Mutex lock 실패(poison)여도 되찾는다(패닉 회피) — 시작 경로라 실질 경합 없음.
        let mut decoder = sync::lock(&self.decoder).take();

        let writer_stop = WriterStop(self.input.clone());

        let handle = std::thread::spawn(move || {
            // ★pump 가 끝나면 라이터도 끝난다★ — `catch_unwind` 바깥이라 정상·panic 두 갈래 모두 지난다.
            let _writer_stop = writer_stop;
            // ★UnwindSafe★: 잡은 stdout/buf/child/shutdown은 panic 후 버려지므로(스레드 종료)
            //   논리 불변 깨짐 없음 → AssertUnwindSafe.
            let normal_reason = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut reader = stdout;
                let mut buf = [0u8; 4096];

                loop {
                    let n = match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };

                    // shutdown 보조 확인 — read가 데이터를 막 반환한 직후 kill이 걸린 경우.
                    //   보통은 write 핸들 close로 인한 EOF가 먼저 깨운다.
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

                // ★break 후 finish 전 flush — 단 kill/shutdown 경로에선 스킵(FIX-2)★:
                //   decoder 는 개행 없이 끝난 잔여 tail 라인을 버퍼에 들고 있을 수 있다. 자연 EOF
                //   (자식이 stdout 을 다 쓰고 정상 종료)라면 이 tail 은 실제로 마지막까지 도착한
                //   완성 데이터일 수 있으므로 finish 전에 마저 뱉어 유실을 막는다.
                //   ★그러나 shutdown(kill) 로 스트림이 **중간에 잘린** 경우엔 flush 하지 않는다★:
                //   그 tail 은 우리가 프로세스를 죽여 truncate 된 조각이라 "완성 라인"이 아니다.
                //   flush(=consume_line 1회)가 우연히 그 잘린 조각을 파싱 가능한 JSON 으로 읽어
                //   가짜 이벤트(예: 부분 result → MessageDone)를 방출할 여지를 원천 차단한다.
                //   실제 도착·개행 종단된 완성 라인은 loop 안 decode 에서 이미 다 emit 됐으므로
                //   스킵해도 실데이터 손실은 없다(버퍼에 남는 건 미종결 tail 뿐).
                //   ★shutdown flag 로 분기하는 이유★: read 레벨에선 자연 EOF 와 kill 이 둘 다
                //   Ok(0) 이라 구분이 안 되지만, kill 은 반드시 shutdown.store(Release) 를 거치므로
                //   여기서 Acquire 로 읽어 확실히 구분된다(아래 reason 산출과 동일 신호원).
                if !shutdown.load(Ordering::Acquire) {
                    if let Some(dec) = decoder.as_mut() {
                        for ev in dec.flush() {
                            pump_core.emit(ev);
                        }
                        pump_core.report_usage(dec.take_usage());
                    }
                }

                // shutdown=true 면 아래서 code 미사용(Killed)이라, 값이 뭐든 무해.
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

            pump_core.finish(reason);

            // G-1: 완료 신호(core.join_pump의 recv_timeout가 받는다). 수신측이 사라졌어도 무시.
            let _ = done_tx.send(());
        });

        core.attach_pump(handle, done_rx);
    }

    /// json 모드에선 이 바이트가 이미 backend가 감싼 stream-json 유저 턴 라인
    /// (`{"type":"user",…}\n`)이다 — transport는 그 형태를 모른다(AgentSession이 InputEncoder로
    /// 감싸 Raw로 넘긴다, ADR-0044 격리).
    /// 큐에 넣고 **즉시** 돌아온다 — OS 쓰기는 전담 라이터 스레드의 일이다.
    ///
    /// ★계약·상한·「받아 둔 뒤의 실패」의 정본은 [`crate::transport::input_queue`] 모듈 헤더★.
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

    /// 주입된 함수가 준 줄을 입력 큐에 넣는다 — 사용자 줄과 같은 라이터 스레드가 **통째로** 쓰므로 줄이 섞이지 않는다.
    /// ★입력 자물쇠(`input_order`)를 타지 않는다★ — 끊기는 입력 id 에 묶이지 않아 그 자물쇠가 지킬 순서가 없고, 그래서
    ///   락 순서에 새 간선이 없다. `Ok` 는 [`Self::send_input`] 과 같이 「받았다」이지 「턴이 멈췄다」가 아니다.
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

    /// ADR-0001 2동사의 파이프판.
    ///
    /// ★kill 인과(파이프판)★: PtyTransport는 master drop→ConPTY close→reader EOF로 pump를 깨우지만,
    ///   파이프는 **자식 트리가 stdout write 핸들을 모두 닫아야** reader가 EOF로 깬다. 그래서
    ///   child.kill + Job terminate(손자 claude까지)로 트리를 통째 죽여 write 핸들을 닫는 것이
    ///   인과의 핵심이다(cmd.exe만 죽이고 claude가 살아 있으면 write 핸들이 안 닫혀 EOF가 안 온다).
    ///
    /// ★순서 불변 — stdin close 는 kill 보다 절대 먼저 오면 안 된다(데드락, FIX 1)★:
    ///   ★한때 여기 「send_input 은 …」으로 적혀 있었다 — 그 락을 쥐는 주체가 바뀌었을 뿐 위험은 그대로다★.
    ///   지금 그 락을 쥐는 것은 **라이터 스레드**다(`write_stdin`, blocking `write_all` 내내). 자식이
    ///   stdin 을 안 읽으면(파이프 backpressure) 그 write_all 이 영원히 블록해 락을 놓지 않는다. 이때 kill **전에**
    ///   `stdin.lock()` 으로 닫으려 하면 그 락을 영영 못 얻어 kill 에 도달조차 못 하고 → pump 가
    ///   깨지 못해 → core.join_pump 가 영구 hang 한다(ADR-0001 인과가 멈춤). 그래서 **kill + Job
    ///   terminate 를 먼저** 한다: 자식을 죽이면 파이프가 깨져 블록된 write_all 이 에러로 풀리고
    ///   락이 해제된다. 그 뒤에야 try_lock 으로 stdin 을 best-effort 정리한다(blocking lock 절대 금지).
    /// ※graceful-exit-via-stdin-close 는 필요 없다 — 어차피 여기서 kill 하므로.
    // ADR-0001
    fn shutdown(&self) {
        // 0. 물러남 표시를 무엇보다 먼저 — `begin_retire` 를 거치지 않는 끝내기 길도 있어서, 표시를 보는 쪽이 아래
        //    종료와 겹치는 창을 여기서도 가장 좁게 둔다. 아래 순서 불변식은 건드리지 않는다(원자 쓰기 하나다).
        self.retiring.store(true, Ordering::Release);

        // 1. shutdown 신호 — pump가 종료 시 Killed로 전이.
        self.shutdown.store(true, Ordering::Release);

        // 1b. 입력 큐를 닫는다 — 라이터 스레드가 이것을 보고 끝난다. ★여기서 잡는 것은 **큐의 락뿐**이라
        //     매달릴 수 없다★: stdin 락에 매달린 라이터는 큐 락을 이미 놓았다(`InputQueue::close` doc).
        //     그래서 이 한 줄은 아래 순서 불변식(kill 먼저)을 건드리지 않는다 — 다른 락이다.
        //     ★대가 = 아직 못 나간 입력은 사라진다★.
        self.input.close("에이전트를 종료했다");

        // 2. wait 는 reap(좀비 방지). 두 번째 호출은 이미 죽어 Err — 무시(멱등).
        {
            let mut child = self.child.lock().expect("child poisoned");
            let _ = child.kill();
            let _ = child.wait();
        }

        // 3. 무리 전체 종료 → 손자(cmd 아래 claude)까지. 무리가 없는 OS(Windows 밖)에서는 무동작이고, child.kill 이
        //    직접 자식(claude, shim 없음)을 죽여 write 핸들이 닫힌다.
        let _ = self.group.terminate(1);

        // 4. try_lock 을 못 얻으면(아직 write_all 이 안 풀린 찰나) 그냥 skip. 미정리 ChildStdin 은
        //    transport drop 시 OS 가 회수하므로 누수 없음(kill 로 이미 파이프는 끊겼다).
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

    // ── 끊기 줄 주입(ADR-0238): 함수가 `Some` 이면 그 줄 한 벌이 부를 것과 함께 큐에 · `None` 이면 `Unsupported` 에 큐 무변경 ──
    // `start()` 를 부르지 않는다 — 라이터가 없어야 큐에 든 것을 그대로 꺼내 잴 수 있다.
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

    // ── 물러남 표시: `begin_retire` 와 `shutdown` 이 각각 세운다 · 무리 손잡이가 내준 읽기 전용 표시도 그것을 보고
    //    통로가 사라진 뒤에도 선 채다 · 예고는 자원을 거두지 않는다 ──
    // 표시는 OS 와 무관하므로 모든 OS 에서 돈다 — 곧 끝나는 무해한 자식으로 통로만 세운다. 무리 손잡이는 Windows
    // 통로에만 있다.
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

    // ── FIX 1 회귀 + 라이터 스레드 회귀 ──
    // 잰다(둘):
    //   (A) ★부른 쪽이 매달리지 않는다★ — 라이터가 파이프 backpressure 로 블록한 **그 상태에서** 온
    //       `send_input` 이 즉시 돌아온다. 이것이 이 라운드가 세운 성질이고, 이 성질이 없으면 데몬의
    //       연결당 dispatch 소비자가 물린 에이전트 하나에 통째로 선다.
    //   (B) FIX 1 그대로 — 그 블록 상태에서 `shutdown` 이 데드락 없이 완료된다. 버그(=stdin close 를
    //       kill 보다 먼저)면 그 락을 영영 못 얻어 hang 하고 아래 데드라인이 잡는다.
    // ★옛 모양과 달라진 곳★: 예전엔 테스트가 직접 띄운 스레드가 `send_input` 안에서 블록했다. 이제
    //   블록하는 것은 **운영 라이터 스레드**라 `start()` 를 반드시 부른다 — 안 부르면 라이터가 없어
    //   아무것도 안 막히고 (A)·(B) 둘 다 공허하게 통과한다.
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
        let (transport, _pid) = StdioTransport::open(&spec, true, None).expect("open");
        let transport = Arc::new(transport);
        let core = Arc::new(OutputCore::new(
            uuid::Uuid::new_v4(),
            0,
            Arc::new(NoopStatusSink) as Arc<dyn StatusSink>,
            TurnWiring::detached(),
        ));
        transport.start(core.clone());

        // 1MiB = 파이프 버퍼를 훨씬 초과 · 큐 상한(2MiB) 아래. 라이터가 이것을 물고 블록한다.
        transport
            .send_input(InputEvent::Raw(vec![b'x'; 1024 * 1024]))
            .expect("상한 아래라 받아야 한다");

        // 라이터가 write_all 에 진입해 stdin 락을 확실히 잡도록 잠깐 양보(넉넉히).
        std::thread::sleep(Duration::from_millis(500));

        // (A) 그 블록 상태에서 온 호출이 **즉시** 돌아온다. 옛 모양이면 여기서 stdin 락을 기다리며
        //     자식이 죽을 때까지(~30s) 매달렸다.
        let probe_start = Instant::now();
        transport
            .send_input(InputEvent::Raw(b"probe\n".to_vec()))
            .expect("큐에 자리가 있으므로 받아야 한다");
        let probe_elapsed = probe_start.elapsed();
        assert!(
            probe_elapsed < Duration::from_secs(2),
            "라이터가 막힌 동안 send_input 이 {probe_elapsed:?} 매달렸다 — 호출자 분리 회귀"
        );

        // (B) FIX 1 — 같은 상태에서 shutdown 이 데드락 없이 완료된다.
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

        // 닫힌 뒤의 입력은 조용히 쌓이지 않고 거절된다.
        assert!(
            matches!(
                transport.send_input(InputEvent::Raw(b"late\n".to_vec())),
                Err(PtyError::WriteFailed(_))
            ),
            "shutdown 뒤의 send_input 은 거절돼야 한다"
        );

        core.join_pump(Duration::from_secs(5));
    }

    // ── 무리 손잡이(ADR-0262): 통로가 사는 동안 멤버가 보이고, 통로가 사라지면 명단은 비고 아무도 못 붙든다 ──
    // `shutdown()` 없이 drop 만 한다 — shutdown 은 Job 을 통째 끝내 손잡이가 강해도 명단이 빈다. drop 만이면
    // 손잡이가 약할 때만 Job 핸들이 닫혀(`KILL_ON_JOB_CLOSE`) 무리가 끝나고, 강하면 ping 이 명단에 남는다. 붙든
    // 멤버도 Job 을 붙들지 않아야 무리가 끝난다 — 뿌리와 ping 을 붙든 채 버린다.
    // `start()` 를 안 불러 통로의 스레드가 없으므로 drop 뒤에 매달리는 것도 없다.
    // cmd 는 stdin 한 줄을 받을 때까지 ping 을 안 띄운다 — `open` 이 Job 에 넣기 전에 ping 이 새어 나가지 않게.
    #[cfg(windows)]
    #[test]
    fn the_process_group_sees_the_root_and_empties_once_the_transport_is_gone() {
        use std::time::{Duration, Instant};

        // ping 이 스스로 끝나는 데 ~9 초 — 아래 5 초 대기 안에 끝나면 그것은 Job 닫기의 몫이다. 출력은 `NUL` 로 보내
        //   통로가 사라지며 파이프가 닫혀 쓰기 실패로 죽는 길도 막는다.
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

        // 콘솔 호스트는 cmd 가 뜨며 붙는다 — 그보다 넉넉히 뒤에 명단을 찍고 문을 열어, 그 명단에 없던 멤버 = ping 으로
        //   가른다(시각을 견주지 않는다).
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
