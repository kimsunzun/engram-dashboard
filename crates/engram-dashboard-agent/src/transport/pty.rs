//! PtyTransport — 콘솔 백엔드(claude/codex/gemini 공용) AgentTransport 구현.
//!
//! tauri import 0. unsafe 0(platform/windows.rs 제외).

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

use crate::output_core::OutputCore;
use crate::transport::input_queue::{self, InputQueue};
use crate::transport::AgentTransport;
use crate::types::{
    CommandSpec, ControlCaps, InputCaps, InputEvent, OutputCaps, OutputEvent, PtyError,
    TerminalReason, TransportCaps,
};

#[cfg(windows)]
use crate::platform::JobObjectHandle;

/// 소유권 분할(fable 저수준 취합 §2): child는 Arc<Mutex>로 pump(try_wait)와 shutdown(kill+wait)이
/// 공유한다. shutdown flag도 Arc — shutdown이 set(Release), pump 종료부가 read(Acquire).
pub struct PtyTransport {
    /// master 는 watcher(자연 종료 감지)와 shutdown(kill) 둘 다 drop(take)할 수 있어 Arc 공유한다.
    /// 둘 다 `take()` 라 멱등 — 먼저 take 한 쪽이 ConPTY 를 닫고, 나중 쪽은 None 을 본다.
    master: Arc<Mutex<Option<Box<dyn MasterPty + Send>>>>,
    /// ★라이터 스레드가 **소유**한다 — `start()` 에서 take 해 move 하고 그 뒤로 여기는 `None` 이다★.
    ///
    /// ★왜 공유(`Arc<Mutex<…>>`)가 아니라 소유인가★: `portable_pty` 의 writer 는 **한 번만 take 할 수
    ///   있고**(트레이트 doc: "It is invalid to take the writer more than once" — unix 는 `bail!`, Windows
    ///   ConPTY 는 `Option::take()`) `Send` 지만 `Sync` 가 아니다. 즉 이 핸들은 **한 스레드의 것**이라야
    ///   하고, 그 스레드가 라이터다. stdio 쪽이 `Arc<Mutex<Option<ChildStdin>>>` 로 공유하는 것과 갈리는
    ///   이유가 이것이다(그쪽은 `shutdown()` 의 `try_lock` 정리 규율이 핸들 공유를 요구한다).
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    /// 아직 못 나간 입력. `send_input` 은 여기 넣고 **즉시** 돌아온다(모듈 = `transport::input_queue`).
    input: Arc<InputQueue>,
    child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
    shutdown: Arc<AtomicBool>,
    /// start()에서 take해 pump로 move. None이면 이미 시작됨.
    reader: Mutex<Option<Box<dyn Read + Send>>>,
    /// ★이 통로가 입력을 **받아들인** 적이 있나 — 「자식이 아직 사용자·모델 내용을 찍은 적이 없다」의
    /// 빗장이다★.
    ///
    /// ★오늘 이 값을 읽는 자리는 테스트뿐이다 — 그래도 지우지 말 것★: 이것이 나르는 사실은 통로만 알
    ///   수 있고(입력 쪽과 출력 쪽을 둘 다 보는 자리가 여기뿐이다) 다시 만들 수 없다. 유일한 운영
    ///   소비자였던 pre-input 관찰 창은 ADR-0217 로 걷혔다.
    /// ★큐가 **받아들인** 순간 올린다 — OS 쓰기를 마친 순간이 아니다★: 지켜야 하는 사실은 큐에 들어선
    ///   순간부터 흔들린다(라이터가 언제 쓸지는 우리가 모른다). 늦게 올리면 그 사이에 읽힌 바이트가
    ///   「입력 전」으로 잘못 세어진다.
    /// ★거절당한 입력으로는 **안 올린다**★ — 큐가 닫혔거나 상한을 넘겨 되돌려 보낸 바이트는 PTY 에
    ///   한 글자도 닿지 않으므로 전제가 그대로다(상한 초과는 실제로 도달 가능한 갈래다 —
    ///   `tests/transport_smoke.rs`).
    /// ★쓰기는 `Release`★ — 읽는 쪽은 `Acquire` 로 받아야 「입력을 받아들였다」와 그것을 관측하는 쪽
    ///   사이에 순서가 선다. `Relaxed` 로 두면 그 주장이 우연한 장벽에 기대게 된다.
    input_seen: Arc<AtomicBool>,
    #[cfg(windows)]
    job_handle: JobObjectHandle,
}

impl PtyTransport {
    /// **pump는 아직 안 띄운다**(start 에서). child_pid 를 함께 반환한다
    /// (claude 세션 추적 부착용 — 호출자가 사용).
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

        // slave는 spawn 후 불필요 — drop으로 FD 누수 방지(닫혀야 ConPTY EOF도 정상).
        drop(pair.slave);

        let child_pid = child.process_id();

        #[cfg(windows)]
        let job_handle = {
            let job = JobObjectHandle::new()?;
            if let Some(pid) = child_pid {
                job.assign(pid)?;
            }
            job
        };

        // ★master를 적재하기 전에 reader/writer를 먼저 확보★.
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
            child: Arc::new(Mutex::new(child)),
            shutdown: Arc::new(AtomicBool::new(false)),
            reader: Mutex::new(Some(reader)),
            input_seen: Arc::new(AtomicBool::new(false)),
            #[cfg(windows)]
            job_handle,
        };

        Ok((transport, child_pid))
    }
}

/// open() 에서 분리해 둔 이유: 터미널 선언 env 의 기본값 주입 규칙이 load-bearing 이라 실제 spawn
/// 없이 단위테스트로 직접 검증할 수 있게 한다.
fn build_pty_command(spec: &CommandSpec) -> CommandBuilder {
    let mut cmd = CommandBuilder::new(&spec.program);
    for a in &spec.args {
        cmd.arg(a);
    }
    cmd.cwd(&spec.cwd);
    // ADR-0184: PTY transport 가 자식에게 터미널 정체성을 선언한다(거부한 대안 넷 = 그 ADR).
    // 데몬은 WMI(WmiPrvSE.exe 가 부모)로 뜨므로 상속 환경이 **서비스 환경**이라 TERM·COLORTERM 이
    //   아예 없다. portable-pty 는 상속 환경 전체(+Windows 는 HKLM/HKCU `Environment` 하이브)를
    //   base env 로 그대로 싣지만 **TERM 을 합성하지는 않는다**(합성하는 건 unix 의 SHELL 뿐).
    //   없으면 codex TUI 가 헤더 박스 아래 테두리와 프롬프트 배경 블록을 잃는다(실측 2026-09-08 —
    //   이 두 변수만 주면 Windows Terminal 과 동일해진다). PTY 를 내주는 이 계층이 곧 터미널이라,
    //   백엔드를 가리지 않고 여기서 "우리가 어떤 터미널인가"를 단언한다.
    // ★상속값은 덮는다★ — 자식은 데몬을 띄운 무언가가 아니라 우리 xterm.js 위젯에 그린다. 상속
    //   TERM 은 그 위젯을 서술하지 않으므로 기본값이 이긴다. 양보 대상은 프로필(spec.env)뿐이다.
    // ADR-0049 의 env 기본값 규율을 그대로 따른다 — ★explicit-skip★: 프로필이 같은 키를 이미 주면
    //   주입하지 않는다(병합 순서 last-wins 에 기대지 않는 결정적 방식). ★대소문자 무시★: Windows
    //   환경변수는 대소문자 무구분이고, unix 에서는 소문자 `term` 이 `TERM` 을 덮는 대신 **둘 다**
    //   자식 블록에 실려 나간다 — eq_ignore_ascii_case 가 그 갈래를 막는다.
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

/// pump 가 **어떤 길로 끝나든** 입력 큐를 닫아 라이터 스레드를 거둔다.
///
/// ★`Drop` 인 것이 요점이다★ — EOF 로 정상 종료하든 pump 본체가 panic 하든(그 갈래는 `catch_unwind` 가
///   흡수한다) 이 자리는 반드시 지난다. codex 통로의 `ReaderExit` 와 같은 규율이고, 같은 이유로 필요하다:
///   ★빠뜨리면 **자연 종료한 에이전트마다 라이터 스레드가 영영 남는다**★. `shutdown()` 은 reaper 경로에서
///   불리지 않으므로 그 스레드를 깨울 다른 자리가 없고, 그 스레드는 큐와 PTY writer 핸들을 문 채 남는다.
struct WriterStop(Arc<InputQueue>);

impl Drop for WriterStop {
    fn drop(&mut self) {
        self.0.close("PTY 스트림이 끝났다 — 더 보낼 곳이 없다");
    }
}

/// B-2. pump 클로저에서 분리해 둔 이유: 이 매핑이 load-bearing(panic→Failed 전이)이라 실제 PTY
/// child 없이 단위테스트로 직접 검증할 수 있게 한다.
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

impl AgentTransport for PtyTransport {
    /// reader가 이미 take됐으면(재호출) 아무것도 안 한다.
    fn start(&self, core: Arc<OutputCore>) {
        let reader = match self.reader.lock().expect("reader poisoned").take() {
            Some(r) => r,
            None => return,
        };

        // ── 입력 라이터 스레드 ──
        // ★아무도 join 하지 않는다★ — `core` 는 pump 핸들 하나만 들고(그 자리를 넓히는 것은 코어 변경이다),
        //   `shutdown()` 안에서 기다리는 것은 ADR-0001 의 2 동사 계약 위반이다. 이 스레드는 큐가 닫히면
        //   스스로 끝나고(`WriterStop` 의 `Drop` 과 `shutdown()` 이 그 둘), 매달린 `write_all` 이 있으면
        //   kill+master drop 이 그것을 에러로 푼다. ★그래서 이 스레드가 kill 인과를 **지연시킬 수 없다**★.
        // ★`start()` 에서 띄우는 이유(= `open()` 이 아니라)★: codex 라이터와 같은 자리에 둬 「통로를 만든
        //   것」과 「통로를 돌리기 시작한 것」을 가른다. `start()` 전에 온 입력은 큐에 그대로 서 있다가
        //   여기서 흘러 나간다 — 부르는 쪽이 준비 여부를 알 필요가 없다(ADR-0190 규율 1).
        if let Some(mut writer) = self.writer.lock().expect("writer poisoned").take() {
            let queue = self.input.clone();
            // ★라이터가 남기는 경고에 귀속을 붙이려면 여기서 id 를 넘겨야 한다★ — 그 안에서는
            //   `core` 를 들 수 없다(스레드로 move 되는 것은 큐와 writer 뿐이다). 아래 spawn 실패
            //   갈래가 이미 `agent = %core.id()` 로 남기는 것과 같은 필드를 쓴다.
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
                // ★조용히 두지 않는다★: 라이터가 없으면 `send_input` 이 계속 `Ok` 를 돌려주면서 바이트는
                //   영영 안 나간다 — 조용한 유실 그 자체다. 큐를 닫아 **그 순간부터 정직하게 거절**한다.
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
        //   exit code 로 한다.
        //
        // ★데드락/이중 wait 안전★: WinChild::try_wait/wait/kill 은 내부 proc 핸들을 try_clone 후
        //   외부 Mutex 를 즉시 해제하므로, watcher 가 우리 child Mutex 를 **짧게만**(try_wait 1회)
        //   잡고 sleep 한다 → shutdown 의 child.lock()+kill+wait 와 경합해도 곧 풀려 데드락 없음.
        //   Windows 는 좀비 reaping 이 없고 핸들 보유 = 종료코드 보존이라, watcher 의 try_wait 와
        //   이후 pump 의 try_wait 가 같은 code 를 반복 회수해도 무해(이중 reap 문제 없음).
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
                    let mut child = match watcher_child.lock() {
                        Ok(g) => g,
                        Err(poisoned) => poisoned.into_inner(),
                    };
                    matches!(child.try_wait(), Ok(Some(_)))
                };
                if exited {
                    let _ = watcher_master.lock().expect("master poisoned").take();
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            })
            .expect("spawn pty watcher thread");
        // watcher 핸들은 detach(join 하지 않음) — kill/자연종료 어느 쪽이든 곧 return 한다.
        drop(watcher);

        let handle = std::thread::spawn(move || {
            // ★pump 가 끝나면 라이터도 끝난다 — 이 guard 가 그 한 몸을 만든다(`WriterStop` doc).★
            //   `catch_unwind` 바깥에 둬 정상·panic 두 갈래 모두에서 drop 이 지나게 한다.
            let _writer_stop = writer_stop;
            // ── B-2: pump 본체를 catch_unwind로 감싼다 ──
            // pump 스레드가 panic하면(emit/read/try_wait 어디서든) 그 agent 출력이 영구 silent
            // 정지하는데 감지·상태전이가 없었다(§5 위반). 본체를 catch_unwind로 잡아, panic이면
            // core.finish(Error)로 Failed 전이시켜 사용자/LLM에게 가시화한다.
            //
            // ★UnwindSafe★: 클로저가 잡는 reader/buf/child/shutdown은 panic 후 더
            //   쓰지 않고 버리므로(스레드가 곧 종료) 논리적 불변 깨짐이 없다 → AssertUnwindSafe로 명시.
            //   Mutex(child) 자체는 UnwindSafe지만 캡처 묶음(특히 dyn Read reader)이 아니므로 감싼다.
            // ★캡처 목록이 **이 에이전트 전용**이라는 것이 아래 poison 범위 논증의 전제다★ — 그래서
            //   **여러 에이전트가 공유하는 자원**(전역 명부 같은)을 이 클로저에 태워 보내면 안 된다.
            //   그 자원의 락이 `expect` 로 풀리는 것이면 이 스레드의 panic 이 곧 다른 에이전트의
            //   재-panic 이 된다.
            // ★이 목록은 캡처가 바뀔 때 **함께** 고친다★ — 낡은 목록은 위 poison 범위 논증을 검증할 수
            //   없는 문장으로 만든다.
            // ★Mutex poison 범위(정확히)★: 아래 reason 산출의 child.lock()만 poison-tolerant
            //   (into_inner)하게 다룬다 — child는 이 transport(=이 agent) 전용이라 그 poison이
            //   다른 agent로 전파되지 않는다.
            //   단 panic이 pump_core.emit() 내부(replay/subscribers lock 보유 중)에서 터지면 그
            //   core Mutex들은 poison되고 poison-tolerant가 아니다 → 이후 그 agent에 새 구독/조회가
            //   오면 subscribe_from/status/snapshot의 .expect("...poisoned")가 재-panic한다. 그러나
            //   (a) core는 agent 전용이라 다른 agent로 전파 안 되고, (b) 그 재-panic은 연결 task
            //   (read_task) 안이라 tokio가 그 task만 격리(데몬·타 agent 무사)한다. 또 현재 emit
            //   경로에는 실제 panic 원이 없다(데몬 출력 sink의 send는 panic 대신 Err 반환). 그래서
            //   core lock은 의도적으로 fail-fast(expect) 유지 — poison은 "데이터 불일치 가능"의
            //   신호라 무시(into_inner)보다 그 agent를 죽이는 게 안전하다.
            let normal_reason = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut reader = reader;
                let mut buf = [0u8; 4096];

                loop {
                    // blocking read — read 자체가 자연 배칭(별도 배치 레이어 없음).
                    let n = match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };

                    // shutdown 보조 확인 — 보통 master drop EOF가 먼저 깨우지만, read가 데이터를
                    //   막 반환한 직후 kill이 걸린 경우를 위한 안전망.
                    if shutdown.load(Ordering::Relaxed) {
                        break;
                    }

                    pump_core.emit(OutputEvent::TerminalBytes(buf[..n].to_vec()));
                }

                // Killed 경로면 안 쓰지만 미리 확보해 둔다.
                // 주: kill 경로는 shutdown의 child.kill()+wait()가 이미 reap했을 수 있어 None일 수 있다
                //     — 그땐 shutdown=true라 Killed로 가므로 code 미사용, 무해.
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

                if shutdown.load(Ordering::Acquire) {
                    TerminalReason::Killed
                } else {
                    TerminalReason::Exited { code }
                }
            }));

            // ★finalize 1회 보존★: panic 경로의 finish와 (정상 EOF 직후 race로) 중복 호출돼도
            //   OutputCore.finalized.swap(AcqRel)가 정확히 1회만 통과시킨다. catch_unwind는 한
            //   클로저가 panic하면 그 안의 정상 finish는 도달 못 하므로, 여기서 정확히 한 번만
            //   finish가 불린다(panic→Error 또는 정상→reason). 이중 호출 자체가 발생하지 않는다.
            let reason = resolve_pump_reason(normal_reason);

            pump_core.finish(reason);

            // G-1: 완료 신호. core.join_pump의 recv_timeout가 받는다. 수신측이 이미
            // 사라졌어도(타임아웃 후 detach) 무시. ★panic 경로에서도 반드시 보낸다★ —
            // catch_unwind로 panic을 흡수했으므로 이 send에 도달한다(join_pump가 5s 안 멈춤).
            let _ = done_tx.send(());
        });

        core.attach_pump(handle, done_rx);
    }

    /// 큐에 넣고 **즉시** 돌아온다 — OS 쓰기는 전담 라이터 스레드의 일이다.
    ///
    /// ★계약·상한·「받아 둔 뒤의 실패」의 정본은 [`crate::transport::input_queue`] 모듈 헤더★. 여기
    ///   되풀어 적지 않는다.
    fn send_input(&self, input: InputEvent) -> Result<(), PtyError> {
        let InputEvent::Raw(bytes) = input;
        // ★받아들인 입력만 빗장을 올린다 — 거절당한 것으로는 안 올린다★(정본 = `input_seen` 필드 doc).
        //   ★`interrupt()` 도 이 자리를 지난다 — 빼지 말 것★: 그쪽도 같은 큐로 실제 바이트(0x03)를
        //   보내므로 「사용자가 아직 아무것도 안 쳤다」가 깨지는 것은 똑같다.
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

    /// ★0x03 은 **같은 FIFO 에 선다** — 큐에 이미 든 입력을 추월하지 않는다★.
    ///
    /// ★그것이 결정이다(현상 유지)★: 큐가 생기기 전에도 인터럽트는 `send_input` 과 **같은 writer 락**을
    ///   지나 도착 순서대로 나갔다. 큐를 들이면서 그 순서를 바꾸면 이 변경이 성능 변경이 아니라 **동작
    ///   변경**이 된다 — 그 판단은 사용자 몫이라 여기서 하지 않는다.
    /// ★codex 통로는 다르다 — 그쪽을 근거로 여기를 고치지 말 것★: 거기서는 인터럽트가 별도 `outbox` 로
    ///   가고 라이터가 그것을 입력 턴보다 **먼저** 집는다. 그럴 근거가 그쪽에만 있다 — 거기서는 거절
    ///   응답이 큐에 선 유저 턴 뒤에서 기다리면 상대가 그 요청의 답을 영영 못 받는다(제어 줄과 데이터가
    ///   같은 JSON-RPC 통로를 공유한다). PTY 에는 그 사실이 없다.
    fn interrupt(&self) -> Result<(), PtyError> {
        self.send_input(InputEvent::Raw(vec![0x03]))
    }

    /// 자원 폐쇄 1~5단계는 **절대순서**다.
    fn shutdown(&self) {
        // 1. shutdown 신호 — pump가 종료 시 Killed로 전이하도록.
        self.shutdown.store(true, Ordering::Release);

        // 1b. 입력 큐를 닫는다 — 라이터 스레드가 이것을 보고 끝난다. ★여기서 잡는 것은 큐의 락뿐이라
        //     매달릴 수 없다★(블로킹 쓰기 중인 라이터는 그 락을 놓고 있다 — `InputQueue::close` doc).
        //     ★대가 = 아직 못 나간 입력은 사라진다★: 이 자식은 다음 줄에서 죽으므로 마저 써도 실패한다.
        self.input.close("에이전트를 종료했다");

        // 2~3. wait 는 reap(좀비 방지). 두 번째 호출은 이미 죽었으니 Err — 무시(멱등).
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

        // 5. master.take() → drop → ClosePseudoConsole → reader EOF — 인과의 핵심.
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

    // ── 터미널 선언 env(TERM/COLORTERM) ──

    fn spec_with_env(env: Vec<(String, String)>) -> CommandSpec {
        CommandSpec {
            program: "cmd.exe".to_string(),
            args: Vec::new(),
            env,
            cwd: std::path::PathBuf::from("."),
        }
    }

    /// ★`get_env` 로 단언하지 말 것★ — 그건 base env(= `std::env::vars_os()` + Windows 레지스트리
    /// 하이브)까지 조회하므로, TERM·COLORTERM 이 이미 있는 개발 셸에서는 **주입 코드를 지워도**
    /// 통과한다(실측). `iter_extra_env_as_str` 는 호출자가 직접 넣은 항목만 돌려줘 "이 코드가
    /// 넣었다"를 잰다. 값 sentinel 도 같은 이유 — 주변 환경이 우연히 만들 수 있는 값(`dumb` 등)을
    /// 쓰면 프로필 우선 단언이 다시 무의미해진다.
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

    /// ADR-0049 대소문자 무시 규율. ★Windows 에서는 `eq_ignore_ascii_case` 를 `==` 로 되돌려도
    /// 이 테스트가 초록이다(실측 2026-09-08)★ — `CommandBuilder` 의 내부 맵이 Windows 에선 키를
    /// 소문자로 접어 `TERM`/`term` 이 어차피 한 항목으로 합쳐지기 때문. 갈라지는 건 unix 로,
    /// 거기선 두 항목이 **둘 다** 자식 블록에 실려 나가 3개가 된다 — 이 단언은 그쪽 회귀망이다.
    /// (주입 자체를 지우는 회귀는 COLORTERM 항목이 사라져 여기서도 잡힌다.)
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

    // ── B-2: panic catch_unwind 결과가 Error reason 으로 매핑되는지 ──
    #[test]
    fn resolve_pump_reason_panic_becomes_error() {
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
    #[test]
    fn panic_reason_finishes_core_as_failed_once() {
        let status_sink = CapturingStatusSink::new();
        let core = Arc::new(OutputCore::new(
            uuid::Uuid::new_v4(),
            0,
            status_sink.clone() as Arc<dyn StatusSink>,
            crate::output_core::TurnWiring::detached(),
        ));

        let panicked =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> TerminalReason {
                panic!("simulated pump panic")
            }));
        let reason = resolve_pump_reason(panicked);
        core.finish(reason);
        // (race 모사) 정상 EOF finish 가 뒤늦게 와도 finalize 1회로 무시.
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

    // ── B-2: 한 agent 의 pump panic 이 다른 agent 로 전파되지 않음(격리) ──
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

        let panicked =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> TerminalReason {
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

/// [`PtyTransport::input_seen`] 빗장의 배선 — ★실 자식 프로세스를 띄운다★.
///
/// ★별도 모듈인 것은 `#[cfg(windows)]` 때문이다★ — `cmd.exe` 를 띄우므로 그 밖의 플랫폼에서는 성립하지
///   않는다. 위 `mod tests` 는 프로세스를 하나도 안 띄우는 것이 그 모듈의 성질이라 섞지 않는다.
/// ★운영 소비자가 없는데도 이 둘이 남는 사유의 정본 = 그 필드 doc★.
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

    /// ★받아들인 첫 입력이 빗장을 올린다★ — 이것이 없으면 「자식이 아직 사용자·모델 내용을 찍은 적이
    /// 없다」를 통로가 더는 말할 수 없게 된다.
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

    /// ★거절당한 입력은 빗장을 올리지 않는다★ — PTY 에 한 글자도 안 닿았으므로 전제가 그대로다.
    #[test]
    fn a_rejected_input_leaves_the_latch_down() {
        let (transport, _pid) = PtyTransport::open(&echo("x"), 80, 24).expect("open");
        // ★`start()` 전에 잰다★ — 라이터가 없어야 큐가 안 빠져 상한 판정이 결정적이다(형제 스위트
        //   `tests/transport_smoke.rs` 의 같은 규율).
        transport
            .send_input(InputEvent::Raw(vec![b'z'; INPUT_QUEUE_MAX_BYTES + 1]))
            .expect_err("상한 초과는 거절돼야 한다");
        assert!(
            !transport.input_seen.load(Ordering::Acquire),
            "거절당한 입력이 빗장을 올렸다"
        );
        transport.shutdown();
        // 큐가 닫힌 뒤의 거절도 같다.
        transport
            .send_input(InputEvent::Raw(b"late".to_vec()))
            .expect_err("닫힌 큐는 거절돼야 한다");
        assert!(
            !transport.input_seen.load(Ordering::Acquire),
            "닫힌 큐의 거절이 빗장을 올렸다"
        );
    }
}

/// ★`shutdown()` 이 **매달린 ConPTY 쓰기를 푼다**는 것을 실측으로 못 박는 자리★.
///
/// ★왜 이 질문이 중요했나★: `portable-pty 0.8.1` 의 `take_writer()` 는 `Inner.writable` 을
///   `Option::take` 하므로(win/conpty.rs) **master drop 이 그 쓰기 핸들을 닫지 않는다** — 핸들은 라이터
///   스레드의 것이다. 아무도 그 스레드를 join 하지 않고 시한도 없으니, 만약 `shutdown()` 이 매달린 쓰기를
///   풀지 못한다면 **죽은 에이전트마다 스레드 하나와 ConPTY 핸들 하나가 영구히 샌다.** 남은 후보는
///   `ClosePseudoConsole` 이 conhost 쪽 읽기 끝을 닫아 주느냐 하나뿐이었고 그것은 OS 비공개 동작이라
///   추론이 아니라 재 보는 수밖에 없었다.
/// ★실측 결론(2026-09-18, 4/4 재현)★: **푼다.** 2MiB 를 문 채 매달린 쓰기가 `shutdown()` 이후
///   14–17ms 만에 `ERROR_BROKEN_PIPE`(os error 109)로 풀리고 라이터가 스스로 끝난다. 그래서 누수 우려는
///   닫혔고, 백스톱(시한·강제 핸들 닫기)을 만들지 않는다. ★이 항목이 그 결론의 회귀망이다★ — 깨지면
///   위 누수가 되살아난 것이므로 백스톱 논의를 다시 열어야 한다.
///
/// ★왜 별도 모듈인가★: 운영 `start()` 는 쓰기 클로저를 자기가 만들므로 밖에서 계측을 끼울 수 없다.
///   대신 `writer` 를 **먼저** 꺼내 계측 클로저로 감싸 우리가 라이터를 띄우고, 그 뒤 `start()` 를 부른다
///   — `start()` 는 `writer` 가 이미 `None` 이라 라이터 spawn 만 건너뛰고 watcher·pump 는 그대로 세우므로
///   `shutdown()` 이 재는 인과(kill → Job terminate → master drop)는 운영과 동일하다.
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
        // ping 은 콘솔 입력을 읽지 않는다 — ConPTY 입력 버퍼가 차면 우리 쪽 write 가 매달린다.
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

        // ★운영보다 **먼저** 꺼낸다★ — 이 한 줄이 계측을 가능하게 하고, 그 대가로 start() 의 라이터
        //   spawn 이 건너뛰어진다(위 모듈 doc).
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

        // ① 실제로 매달렸나 — 이것이 참이어야 아래 판정이 공허하지 않다.
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

        // ② shutdown 이 그것을 푸는가 — 실측 14–17ms.
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
