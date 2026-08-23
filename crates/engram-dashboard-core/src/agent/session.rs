//! AgentSession — 에이전트 1개 = OutputCore(출력 측) + Box<dyn AgentTransport>(채널/자원 측) 합성.
//!
//! transport 종류(PTY/API)와 무관한 공용 표면을 노출하고, 내부에서 core/transport로 위임한다.
//!
//! 소유권 분할(impl-spec 표): master/child/shutdown/job/reader/writer → transport(PtyTransport) 안,
//!   subscribers/replay/seq/status/finalized → core(OutputCore) 안.
//!
//! 따라서 모든 메서드는 자기 필드(cols/rows atomic)를 만지거나 core/transport로 위임할 뿐이다.
//!
//! tauri import 0.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU16, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::agent::backend::InputEncoder;
use crate::agent::output_core::OutputCore;
use crate::agent::transport::AgentTransport;
use crate::agent::types::{
    AgentId, AgentStatus, BackendCaps, Capabilities, InputEvent, OutputChunk, OutputSink, PtyError,
    SinkId, SubscribeOutcome, TerminationIntent, WriteOutcome,
};

pub struct AgentSession {
    pub id: AgentId,
    pub cwd: PathBuf,
    pub epoch: u32,
    pub cols: AtomicU16,
    pub rows: AtomicU16,
    /// 유저 종료 의도. `Arc` 인 이유: finalize hook 클로저가 같은 값을 공유 캡처한다.
    intent: Arc<AtomicU8>,
    /// backend(프로그램)가 결정한 caps(session/model). manager.spawn 이 profile.command 로 산출해
    /// 주입한다 — transport 는 이 값을 모른다.
    backend_caps: BackendCaps,
    /// write_input 을 transport 로 넘기기 **직전** 적용하는 입력 인코딩(ADR-0044/0004).
    /// manager.spawn 이 산출해 주입한다.
    encoder: InputEncoder,
    /// 이 세션이 **편지를 읽는 주체**인가(= 우편 수신자 명단 자격). spawn 시 backend 가 산출한 값을
    /// 그대로 들고 있는다 — encoder 와 같은 부류의 backend 파생 사실이다.
    ///
    /// ★프로필이 아니라 **세션**이 드는 이유(load-bearing)★: `DeleteProfile` 은 산 세션을 죽이지
    ///   않는다. 프로필로 판정하면 프로필이 지워진 산 셸이 "모름" 이 되어 명단에 되돌아오고, 봉투가
    ///   명령으로 실행된다. 세션은 자기가 무엇으로 spawn 됐는지 알고 그 사실은 프로필 삭제로 안 변한다.
    reads_messages: bool,
    /// 본문 write 와 제출 write 사이의 대기 — 근거·값 출처는 `backend::SUBMIT_PACING`.
    ///
    /// ★기본값은 `new` 가 박는다(생성자 인자가 아니다)★: 호출자가 값을 고를 수 있게 하면 어느 조립
    ///   경로 하나가 0 을 넘기는 순간 이 결함이 조용히 재발한다. 낮추는 길은 테스트 전용 seam 하나뿐이다.
    submit_pacing: Duration,
    /// 위 대기를 실제로 재우는 함수. 운영은 블로킹 sleep 이고, 테스트는 **재우지 않고 호출만 기록**하는
    /// 것을 꽂아 "대기가 발행됐다" 를 시간 측정 없이(= 비플래키) 단언한다.
    sleeper: fn(Duration),
    core: Arc<OutputCore>,
    transport: Box<dyn AgentTransport>,
}

/// 운영 sleeper — 이 층은 동기 경로라 그냥 블로킹으로 잔다(배달 루프가 그만큼 붙잡히는 것은 수용한 성질:
/// 터미널 수신은 시연 용도이고, 비동기 분리는 구조 변경이라 별도 결정 사항이다).
fn blocking_sleep(d: Duration) {
    std::thread::sleep(d);
}

impl AgentSession {
    /// 합성 세션 생성. **start는 여기서 호출하지 않는다** — manager가 new 이전에
    /// `transport.start(core.clone())`를 직접 부른다(impl-spec: 테스트 가시성·spawn 흐름 명시성).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: AgentId,
        cwd: PathBuf,
        epoch: u32,
        cols: u16,
        rows: u16,
        intent: Arc<AtomicU8>,
        backend_caps: BackendCaps,
        encoder: InputEncoder,
        reads_messages: bool,
        core: Arc<OutputCore>,
        transport: Box<dyn AgentTransport>,
    ) -> Self {
        Self {
            id,
            cwd,
            epoch,
            cols: AtomicU16::new(cols),
            rows: AtomicU16::new(rows),
            intent,
            backend_caps,
            encoder,
            reads_messages,
            submit_pacing: crate::agent::backend::SUBMIT_PACING,
            sleeper: blocking_sleep,
            core,
            transport,
        }
    }

    /// ★테스트 전용 seam★ — 제출 대기를 낮추거나(하네스가 0.5초씩 자지 않게) 대기 발행 자체를 관측한다.
    /// **운영 기본값은 언제나 `backend::SUBMIT_PACING`**(위 필드 doc) — 이 함수는 그 기본값을 바꾸지 않고,
    /// 테스트가 자기 인스턴스에 대해서만 명시로 내린다.
    #[cfg(test)]
    pub(crate) fn with_submit_pacing(mut self, pacing: Duration, sleeper: fn(Duration)) -> Self {
        self.submit_pacing = pacing;
        self.sleeper = sleeper;
        self
    }

    /// 이 세션이 편지를 읽는 주체인가 — 판정 근거는 필드 doc.
    pub fn reads_messages(&self) -> bool {
        self.reads_messages
    }

    /// 유저 종료 의도 태깅(ADR-0019) — kill_agent 가 transport.shutdown **전에** 호출한다.
    /// finish hook 이 이 값을 finish 순간 snapshot 하므로, shutdown 전에 set 해야 pump 가
    /// 깨어 finish 할 때 UserKill 이 관측된다(순서가 race 방지의 핵심).
    pub fn set_intent(&self, intent: TerminationIntent) {
        self.intent.store(intent as u8, Ordering::SeqCst);
    }

    /// pump 기동을 위임(transport.start). ★ADR-0019 reaper 순서★: manager 는 이 세션을 sessions
    /// 맵에 **insert 한 뒤** start 한다. pump 가 즉시 EOF→finish→ReapMsg 를 보내도 그땐 이미 맵에
    /// 존재하므로 reaper 가 정상 reap 한다(insert 전 start 면 hook send 가 맵에 없는 id 를 가리켜
    /// 좀비). attach_pump 는 start 내부 동기 완료라 insert 순서와 무관(join_pump 영향 없음).
    pub fn start_pump(&self) {
        self.transport.start(self.core.clone());
    }

    /// 입력 바이트 전달 → (encoder 적용) → transport.
    ///
    /// ★배선 지점(ADR-0044)★: encoder를 적용해 텍스트 턴을 백엔드 규약대로 감싼 뒤
    ///   **항상 Raw 바이트**로 transport에 넘긴다. transport는 바보 파이프라 형태를 모른다.
    ///   - Raw(터미널·shell): `encode`가 바이트를 그대로 복사 → 기존 경로와 **바이트 동일**.
    ///   - ClaudeStreamJson(json 모드): 텍스트를 claude 유저 JSON 라인으로 감싼다(escape·스키마는
    ///     backend/claude.rs 단독 — session은 태그만 들고 형태를 모른다, ADR-0004 격리).
    ///
    /// ★호출 계약(FIX 6a) — json 모드에서 `1 write_input 호출 == 완결된 유저 턴 1개`★:
    ///   ClaudeStreamJson 인코더는 매 호출을 `{"type":"user",…}\n` 라인 **하나**로 감싼다. 즉 호출
    ///   1회당 claude 는 유저 턴 1개를 통째로 받는다. 터미널 경로처럼 **키 입력 1글자씩** 호출하면
    ///   글자마다 한 글자짜리 잘못된 턴이 만들어져 대화가 깨진다. 따라서 json 모드 호출자(RichSlot·M2)는
    ///   **완성된 메시지 전체를 한 번에** 보내야 한다(부분 입력 누적은 프론트 입력창 몫). 터미널 경로는
    ///   Raw 라 기존대로 스트리밍 바이트 호출이 정상(이 계약은 json 모드 한정).
    pub fn write_input(&self, bytes: &[u8]) -> Result<(), PtyError> {
        self.write_input_observed(bytes).map(|_| ())
    }

    /// `write_input` 의 배달-경계 계측판(ADR-0088 Stage 0) — 성공 시 `WriteOutcome`(논리 메시지 바이트 +
    ///   이 턴의 `msg_uuid`)을 돌려준다. 동작·바이트는 `write_input` 과 **완전히 동일**하고(같은 본체),
    ///   차이는 **관측 산출물을 삼키지 않고 반환**하는 것뿐이다. 제어 채널 relay(ingress::handle_send)가
    ///   이 산출물로 배달 관측 레코드를 만든다("전송 실패" vs "모델 무시" 구별의 전제 — ADR-0088).
    ///
    /// ★완결성 = Ok-vs-Err★: `send_input` 이 `Ok(())` 를 돌려주면(내부 `write_all`) 요청 바이트가 전량
    ///   수용된 것이다(std write_all 계약 — 부분 write 를 `Ok` 로 숨기지 않음). 전량 미수용은 이 함수가
    ///   `Err` 로 반환하지 `Ok` 로 축소 반환하지 않는다. `WriteOutcome` 의 바이트 필드는 완결성 판정
    ///   레버가 아니다(이유는 `WriteOutcome` 주석) — 완결성은 이 함수의 `Ok`/`Err` 로 본다.
    // ADR-0088
    pub fn write_input_observed(&self, bytes: &[u8]) -> Result<WriteOutcome, PtyError> {
        // ★이 유저 턴의 메시지 uuid(replay dedup 키)★: 한 write_input 당 하나 생성해 (a) stdin user
        //   라인(encode)과 (b) 입력-시점 합성 에코(input_echo_event) **양쪽에 같은 값**으로 넘긴다.
        //   json 모드에서 claude 가 replay 로 이 uuid 를 그대로 되울린다(실측). session 은 불투명 Uuid
        //   토큰만 알고 json 형태·uuid 부착 위치는 모른다(ADR-0004 격리 — 스키마 지식은
        //   backend/claude.rs 단독). Raw(터미널) encoder 는 이 uuid 를 무시한다.
        let msg_uuid = uuid::Uuid::new_v4();
        let encoded = self.encoder.encode(bytes, msg_uuid);
        self.transport.send_input(InputEvent::Raw(encoded))?;

        // ★ADR-0044/0045 · 왜: 입력-시점 유저 에코★: 터미널(Raw)은 PTY 가 입력을 즉시 로컬 에코하지만,
        //   json(stream-json) 모드는 claude 가 `--replay-user-messages` 로 되울릴 때까지(왕복 지연)
        //   유저 메시지가 화면에 안 뜬다. 그래서 send_input **성공 후**, encoder 가 json 모드면 동일한
        //   유저 이벤트를 즉시 core.emit 해 터미널의 즉시 에코를 흉내낸다(체감 반응성). 이후 claude 가
        //   되울린 replay 중복은 프론트 accumulator 가 uuid 로 dedup 한다(같은 msg_uuid) — decoder 는
        //   억제하지 않고 uuid 를 실어 그대로 통과시킨다(backend/claude.rs). 과거/비매칭 uuid 의 user
        //   text(resume 재개분)는 dedup 되지 않아 전부 보존된다(vanish 회귀 제거).
        //   ★락 규율(ADR-0006)★: 새 락 없이 core.emit 재사용 — emit 이 replay/subscribers 락을 짧게만
        //   잡고 lock 밖 send 하는 규율을 그대로 탄다. send_input 성공 후 emit 이라 순서도 자연스럽다.
        if let Some(event) = self.encoder.input_echo_event(bytes, msg_uuid) {
            self.core.emit(event);
        }
        let n = bytes.len();
        Ok(WriteOutcome {
            bytes_requested: n,
            bytes_written: n,
            msg_uuid,
            epoch: self.epoch,
        })
    }

    /// `write_input_observed` + **제출**: 본문을 쓴 뒤, encoder 가 제출 바이트를 요구하면 그것을
    /// **별도 write** 로 한 번 더 낸다. 반환값·유저 에코·바이트 회계는 `write_input_observed` 와 동일하다
    /// (제출 바이트는 논리 메시지가 아니라 `WriteOutcome` 에 세지 않는다).
    ///
    /// ★왜 두 번 쓰나 · 왜 encoder 가 못 합치나★ = `InputEncoder::submit_sequence` 주석(실측 근거).
    /// ★두 write 사이에 `backend::SUBMIT_PACING` 만큼 잔다★: 나눠 쓰는 것만으로는 부족하고, 간격이
    ///   없으면 PTY 에서 한 덩이로 묶여 수신자가 한 번의 read 로 받는다(그 상수 doc — "0ms 로 된다" 는
    ///   옛 관측은 측정 오류였다). **그래서 이 동사는 호출 스레드를 그만큼 붙잡는다.**
    /// ★왜 이 층인가★: transport 를 소유해 `send_input` 을 두 번 낼 수 있는 가장 낮은 층이 여기다. 위층
    ///   (manager·데몬 어댑터·메시징 커널)은 transport 를 모르고, 아래층(encoder)은 바이트열 하나를
    ///   돌려주는 계약이라 write 경계를 만들 수 없다.
    /// ★`write_input` 과 갈라 둔 이유★: 터미널 키 입력은 사람이 Enter 를 직접 친다 — 그 경로가 이 동사를
    ///   타면 키 한 번마다 턴이 제출된다. 이 동사는 "완성된 메시지 하나를 턴으로 넣는" 호출자(우편 배달)
    ///   전용이다.
    /// ★에러 계약★: 본문은 갔는데 제출 write 가 실패하면 `Err` 다 — 턴이 시작되지 않은 배달은 배달이
    ///   아니므로 상위(파킹·재시도)가 실패로 다뤄야 한다.
    pub fn submit_input_observed(&self, bytes: &[u8]) -> Result<WriteOutcome, PtyError> {
        let outcome = self.write_input_observed(bytes)?;
        if let Some(submit) = self.encoder.submit_sequence() {
            // ★이 대기가 제출의 일부다(빼면 제출되지 않는다 — 실측)★: 근거·값 출처·"0ms 로 된다" 는
            //   옛 관측이 왜 틀렸는지는 `backend::SUBMIT_PACING` doc.
            (self.sleeper)(self.submit_pacing);
            //
            // ★두 실패를 로그에서 가른다(본문도 못 감 vs 본문은 갔고 제출만 실패)★: 후자는 수신자
            //   입력창에 미제출 봉투가 남은 상태라, 상위의 무손실 재파킹이 다음 flush 에서 같은 봉투를
            //   그 위에 덧쓴다(한 턴에 두 벌). 입력창을 비우는 동사는 이 층의 계약 밖이라 지우지는
            //   못하고, 사람이 그 잔여물을 알아볼 수 있게 남기는 것이 여기서 할 수 있는 전부다.
            if let Err(e) = self.transport.send_input(InputEvent::Raw(submit.to_vec())) {
                tracing::warn!(
                    agent = %self.id,
                    epoch = self.epoch,
                    bytes = bytes.len(),
                    "본문은 썼으나 제출 write 실패 — 수신자 입력창에 미제출 봉투가 남았고 재시도가 그 위에 덧쓴다: {e}"
                );
                return Err(e);
            }
        }
        Ok(outcome)
    }

    /// transport.resize 성공 후에만 cols/rows atomic 을 갱신한다 — 실패 시 옛 값 유지.
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), PtyError> {
        self.transport.resize(cols, rows)?;
        self.cols.store(cols, Ordering::Relaxed);
        self.rows.store(rows, Ordering::Relaxed);
        Ok(())
    }

    /// 진행 중 작업만 중단(≠kill — 프로세스는 살아 있다). PTY=0x03 주입.
    pub fn interrupt(&self) -> Result<(), PtyError> {
        self.transport.interrupt()
    }

    /// 자원 강제 종료 + pump 종료 대기. **이 2동사 순서(shutdown THEN join_pump)가 kill 인과의 핵심.**
    /// shutdown이 master를 drop해 pump read를 EOF로 깨우고(→core.finish(Killed)), join_pump가
    /// 그 pump 종료를 기다린다. 역전 시 hang(아직 살아있는 pump를 기다림).
    pub fn kill(&self, timeout: Duration) {
        self.transport.shutdown();
        self.core.join_pump(timeout);
    }

    /// 과도기 Exiting 전이 — kill 직전 manager가 먼저 호출(stage 6). enter_exiting과 kill은
    /// 별개 동사다. terminal(이미 종료)이면 false.
    pub fn enter_exiting(&self) -> bool {
        self.core.enter_exiting()
    }

    /// 최종 capability — transport(물리: input/output/control)와 backend(프로그램: session/model)
    /// 의 합성. 출처가 타입으로 분리돼 있어 transport 가 resume 을, backend 가 resize 를 섞어
    /// 채우는 사고가 구조적으로 불가능하다.
    pub fn capabilities(&self) -> Capabilities {
        Capabilities::compose(self.transport.capabilities(), self.backend_caps.clone())
    }

    pub fn subscribe(&self, sink: Arc<dyn OutputSink>) -> SinkId {
        self.core.subscribe(sink)
    }

    /// `on_ready`: replay 전송 직전(subscribers lock 보유 중) 1회 호출 — core 위임(불변식 2/TOCTOU).
    pub fn subscribe_from(
        &self,
        sink: Arc<dyn OutputSink>,
        after_seq: Option<u64>,
        epoch_matches: bool,
        on_ready: impl FnOnce(&SubscribeOutcome),
    ) -> SubscribeOutcome {
        self.core
            .subscribe_from(sink, after_seq, epoch_matches, on_ready)
    }

    pub fn unsubscribe(&self, sink_id: SinkId) {
        self.core.unsubscribe(sink_id);
    }

    pub fn snapshot(&self) -> Vec<OutputChunk> {
        self.core.snapshot()
    }

    /// 마지막 콘솔 바이트 최대 `max_bytes`(계약·비용은 `OutputCore::terminal_tail`).
    // ADR-0169
    pub fn terminal_tail(&self, max_bytes: usize) -> Vec<u8> {
        self.core.terminal_tail(max_bytes)
    }

    /// 이 화신이 낸 **진단(stderr) 텍스트** 꼬리(계약·상한은 `OutputCore::diagnostic_tail`).
    ///
    /// ★`terminal_tail` 의 대체재가 아니라 짝이다★: 두 스트림은 transport 에 따라 배타적으로 찬다
    ///   — 파이프(구조화) 세션은 여기가 차고 링은 비고, PTY(터미널) 세션은 반대다. 그래서 분류
    ///   호출자는 둘 중 하나를 고르지 않고 **합쳐서** 넘긴다.
    // ADR-0169
    pub fn diagnostic_tail(&self) -> String {
        self.core.diagnostic_tail()
    }

    pub fn status(&self) -> AgentStatus {
        self.core.status()
    }

    pub fn cols(&self) -> u16 {
        self.cols.load(Ordering::Relaxed)
    }

    pub fn rows(&self) -> u16 {
        self.rows.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::backend::{AgentBackend, ClaudeBackend, ShellBackend};
    use crate::agent::transport::stdio::StdioTransport;
    use crate::agent::types::{ControlCaps, InputCaps, OutputCaps, TransportCaps};
    use std::sync::Mutex;

    /// 실 프로세스 없이 인코딩 배선을 단언하기 위한 격리 하네스(ADR-0012).
    /// `fail_from`: 이 순번(0-based)부터의 write 를 실패시킨다 — 본문은 성공하고 제출만 실패하는 갈래를
    ///   만들기 위한 것이다(`None` = 전부 성공).
    struct CapturingTransport {
        captured: Arc<Mutex<Vec<Vec<u8>>>>,
        fail_from: Option<usize>,
    }
    impl AgentTransport for CapturingTransport {
        fn start(&self, _core: Arc<OutputCore>) {}
        fn send_input(&self, input: InputEvent) -> Result<(), PtyError> {
            let InputEvent::Raw(bytes) = input;
            let mut captured = self.captured.lock().unwrap();
            if self.fail_from.is_some_and(|n| captured.len() >= n) {
                return Err(PtyError::WriteFailed("harness: write refused".into()));
            }
            captured.push(bytes);
            Ok(())
        }
        fn resize(&self, _cols: u16, _rows: u16) -> Result<(), PtyError> {
            Ok(())
        }
        fn interrupt(&self) -> Result<(), PtyError> {
            Ok(())
        }
        fn shutdown(&self) {}
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
                    resize: false,
                    interrupt: false,
                    cancel: false,
                    graceful_shutdown: false,
                },
            }
        }
    }

    struct NoopStatusSink;
    impl crate::agent::types::StatusSink for NoopStatusSink {
        fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
        fn agent_list_updated(&self, _agents: Vec<crate::agent::types::AgentInfo>) {}
    }

    /// 실 프로세스 없이 입력-시점 유저 에코 emit(ADR-0044/0045)을 단언하기 위한 하네스.
    /// 수집 태그: `Structured` 는 `"structured:<kind>"`, 그 외는 variant 명.
    struct EmitCapturingSink {
        id: SinkId,
        seen: Arc<Mutex<Vec<String>>>,
    }
    impl OutputSink for EmitCapturingSink {
        fn send(
            &self,
            frame: crate::agent::types::OutputFrame<'_>,
        ) -> Result<(), crate::agent::types::SinkError> {
            use crate::agent::types::{OutputEvent, OutputPayload};
            if let OutputPayload::Event(e) = frame.payload {
                let tag = match e {
                    OutputEvent::Structured { kind, .. } => format!("structured:{kind}"),
                    other => format!("{other:?}"),
                };
                self.seen.lock().unwrap().push(tag);
            }
            Ok(())
        }
        fn sink_id(&self) -> SinkId {
            self.id
        }
    }

    fn session_with(encoder: InputEncoder) -> (AgentSession, Arc<Mutex<Vec<Vec<u8>>>>) {
        session_harness(encoder, None)
    }

    fn session_failing_after(
        encoder: InputEncoder,
        ok_writes: usize,
    ) -> (AgentSession, Arc<Mutex<Vec<Vec<u8>>>>) {
        session_harness(encoder, Some(ok_writes))
    }

    fn session_harness(
        encoder: InputEncoder,
        fail_from: Option<usize>,
    ) -> (AgentSession, Arc<Mutex<Vec<Vec<u8>>>>) {
        let id = uuid::Uuid::new_v4();
        let core = Arc::new(OutputCore::new(
            id,
            0,
            Arc::new(NoopStatusSink),
            crate::agent::output_core::TurnWiring::detached(),
        ));
        let captured = Arc::new(Mutex::new(Vec::new()));
        let transport = Box::new(CapturingTransport {
            captured: captured.clone(),
            fail_from,
        });
        let shell_cmd = crate::agent::profile::AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec![],
        };
        let session = AgentSession::new(
            id,
            PathBuf::from("."),
            0,
            80,
            24,
            Arc::new(AtomicU8::new(0)),
            ShellBackend.capabilities(&shell_cmd),
            encoder,
            true,
            core,
            transport,
        )
        // 하네스는 안 잔다 — 대기 **발행 여부**는 아래 전용 테스트가 sleeper 호출로 단언한다(시간
        //   측정 없이). 여기서 실제로 자면 단위 테스트가 호출마다 0.5초씩 붙잡힌다.
        .with_submit_pacing(Duration::ZERO, |_| {});
        (session, captured)
    }

    // ── Raw 인코더(터미널 경로 회귀 불변) ──
    #[test]
    fn write_input_raw_is_byte_identical() {
        let (session, captured) = session_with(InputEncoder::Raw);
        let input = b"echo hi\r\n\x1b[A\x03";
        session.write_input(input).unwrap();
        let got = captured.lock().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0], input.to_vec(), "Raw 는 바이트 동일이어야 함");
    }

    // ── ADR-0088: 배달-경계 계측 ──
    #[test]
    fn write_input_observed_surfaces_bytes_and_msg_uuid() {
        let (session, captured) = session_with(InputEncoder::Raw);
        let input = b"hello-observed"; // 14바이트 ASCII.
        let outcome = session
            .write_input_observed(input)
            .expect("write_input_observed ok");
        // ★FIX-5★: off-by-one·계층 회귀를 거르는 exact 카운트 단언.
        assert_eq!(
            outcome.bytes_requested, 14,
            "요청 바이트 = 넘긴 입력의 정확 바이트 수"
        );
        assert_eq!(outcome.bytes_requested, input.len(), "요청 = 입력 len");
        assert_eq!(
            outcome.bytes_written, outcome.bytes_requested,
            "by-construction 항등(bytes_written = bytes_requested 복사) — short-write 탐지 아님"
        );
        assert!(
            !outcome.msg_uuid.is_nil(),
            "이 유저 턴의 msg_uuid 를 노출해야(상관 키)"
        );
        assert_eq!(
            outcome.epoch, 0,
            "WriteOutcome.epoch = write 를 수행한 세션의 epoch(by-construction)"
        );
        // 계측판이 Raw 바이트 동일성을 깨지 않는다.
        assert_eq!(captured.lock().unwrap()[0], input.to_vec());
    }

    // ── ADR-0088(FIX-5): 멀티바이트(UTF-8) 본체 ──
    #[test]
    fn write_input_observed_counts_bytes_not_chars_multibyte() {
        // "안녕" = 한글 2자, 각 3바이트 UTF-8 = 6바이트. char 수(2)로 세면 여기서 깨진다.
        let (session, _captured) = session_with(InputEncoder::Raw);
        let input = "안녕".as_bytes();
        assert_eq!(input.len(), 6, "UTF-8 로 6바이트여야(테스트 전제)");
        let outcome = session
            .write_input_observed(input)
            .expect("write_input_observed ok");
        assert_eq!(
            outcome.bytes_requested, 6,
            "멀티바이트 요청은 char 수(2)가 아니라 바이트 수(6)여야"
        );
        assert_eq!(
            outcome.bytes_written, 6,
            "by-construction 복사도 바이트 수(6)"
        );
    }

    // ── 제출 분리(submit_input_observed) ──
    //
    // ★회귀 축은 바이트가 아니라 **write 경계**다★: 봉투 바이트는 예전에도 PTY 에 도착했지만 턴이 시작되지
    //   않았다. `본문+CR` 을 한 write 로 합치면 claude TUI 가 제출하지 않고, 두 write 로 나누면 제출된다
    //   (실측 2026-08-17). 그래서 아래 단언은 write **횟수와 경계**를 본다 — 이어붙인 바이트를 보지 않는다.

    #[test]
    fn submit_input_terminal_writes_the_body_then_the_submit_byte_separately() {
        let (session, captured) = session_with(InputEncoder::Raw);
        let body = b"<message from=\"bob\">hi</message>";

        let outcome = session
            .submit_input_observed(body)
            .expect("submit_input_observed ok");

        let got = captured.lock().unwrap();
        assert_eq!(
            got.len(),
            2,
            "본문과 제출은 분리된 write 여야(한 write 로 합치면 TUI 가 제출하지 않음): {got:?}"
        );
        assert_eq!(
            got[0],
            body.to_vec(),
            "본문 write 는 봉투 바이트 그대로 — 제출 바이트를 섞지 않는다"
        );
        assert_eq!(got[1], b"\r".to_vec(), "두 번째 write = 제출(CR) 단독");
        assert_eq!(
            outcome.bytes_requested,
            body.len(),
            "회계는 논리 메시지 바이트만 — 제출 바이트는 세지 않는다"
        );
        assert_eq!(outcome.bytes_written, body.len());
    }

    #[test]
    fn submit_input_json_mode_writes_exactly_the_encoded_line_and_nothing_else() {
        let (session, captured) = session_with(InputEncoder::ClaudeStreamJson);
        let body = b"hello";

        let outcome = session
            .submit_input_observed(body)
            .expect("submit_input_observed ok");

        // ★exact 비교★: 돌려받은 msg_uuid 로 기대 라인을 재구성해 **바이트 정확 일치**를 본다. 제출 write 를
        //   더하거나 봉투에 문자를 덧붙이는 회귀는 전부 여기서 걸린다("CR 이 없다" 류의 약한 단언과 달리
        //   무엇이 추가돼도 잡힌다). `write_input` 과의 동일성을 직접 비교하지 않는 이유 = 그쪽은 자체
        //   msg_uuid 를 새로 뽑아 두 산출물이 uuid 만큼 다르기 때문이다.
        let expected = InputEncoder::ClaudeStreamJson.encode(body, outcome.msg_uuid);
        let got = captured.lock().unwrap();
        assert_eq!(
            got.len(),
            1,
            "json 경로는 종전 그대로 write 1회 — 제출 write 를 더하면 CR 만 든 빈 턴이 하나 더 생긴다: {got:?}"
        );
        assert_eq!(
            got[0],
            expected,
            "인코더 산출물과 바이트 정확 일치여야: {}",
            String::from_utf8_lossy(&got[0])
        );
    }

    /// ★대기가 **실제로 발행되는지** + 운영 기본값이 무엇인지를 함께 못 박는다★.
    ///
    /// 이 결함의 본체는 write 횟수가 아니라 수신자의 read 경계였다 — 나눠 써도 간격이 없으면 한 덩이로
    /// 묶여 제출되지 않는다(`backend::SUBMIT_PACING` doc). 그래서 "제출 write 전에 대기가 있었나" 와
    /// "그 값이 운영 기본값인가" 둘 다 회귀 축이다. 시간을 재지 않고 sleeper 호출을 기록해 단언하므로
    /// 플래키하지 않고, 테스트가 0.5초를 실제로 자지도 않는다.
    #[test]
    fn submit_input_waits_between_the_body_and_the_submit_write() {
        use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
        // 이 테스트 전용 기록판 — 다른 테스트의 하네스는 no-op sleeper 를 쓰므로 여기 안 닿는다.
        static SLEPT_MICROS: AtomicU64 = AtomicU64::new(0);
        static CALLS: AtomicU64 = AtomicU64::new(0);
        fn recording_sleep(d: Duration) {
            SLEPT_MICROS.store(d.as_micros() as u64, AtomicOrdering::SeqCst);
            CALLS.fetch_add(1, AtomicOrdering::SeqCst);
        }

        let (session, captured) = session_harness(InputEncoder::Raw, None);
        // ★운영 기본값 그대로 두고 재우는 함수만 바꾼다★: 값까지 테스트가 정하면 기본값 회귀를 못 본다.
        let session =
            session.with_submit_pacing(crate::agent::backend::SUBMIT_PACING, recording_sleep);

        session
            .submit_input_observed(b"envelope")
            .expect("submit ok");

        assert_eq!(
            CALLS.load(AtomicOrdering::SeqCst),
            1,
            "본문과 제출 사이에 대기가 정확히 한 번 발행돼야(빠지면 두 write 가 한 read 로 묶인다)"
        );
        assert_eq!(
            SLEPT_MICROS.load(AtomicOrdering::SeqCst),
            crate::agent::backend::SUBMIT_PACING.as_micros() as u64,
            "대기 값 = 운영 기본값(줄이면 이 결함이 재발한다 — 상수 doc 의 근거를 먼저 읽을 것)"
        );
        assert!(
            crate::agent::backend::SUBMIT_PACING > Duration::ZERO,
            "★운영 기본값이 0 이 되는 형태 금지★ — 0 이면 나눠 쓴 의미가 사라진다"
        );
        assert_eq!(
            captured.lock().unwrap().len(),
            2,
            "대기를 넣어도 write 는 여전히 본문 + 제출 둘"
        );
    }

    #[test]
    fn submit_input_reports_a_failure_that_only_hit_the_submit_write() {
        // 본문은 이미 수신자 입력창에 들어간 뒤 제출만 실패하는 갈래 — 상위가 재파킹으로 다루려면
        // 성공으로 삼켜지면 안 된다(그 잔여물의 의미는 submit_input_observed doc).
        let (session, captured) = session_failing_after(InputEncoder::Raw, 1);

        let err = session.submit_input_observed(b"envelope");

        assert!(
            matches!(err, Err(PtyError::WriteFailed(_))),
            "제출 write 실패는 Err 로 표면화돼야: {err:?}"
        );
        assert_eq!(
            captured.lock().unwrap().as_slice(),
            &[b"envelope".to_vec()],
            "본문은 이미 나갔다 — 그래서 재시도가 같은 봉투를 덧쓰는 잔여물이 남는다"
        );
    }

    #[test]
    fn write_input_does_not_submit() {
        // 사람이 Enter 를 직접 치는 키 입력 경로 — 여기에 제출이 끼면 키 한 번마다 턴이 나간다.
        let (session, captured) = session_with(InputEncoder::Raw);
        session.write_input(b"partial").unwrap();
        let got = captured.lock().unwrap();
        assert_eq!(got.len(), 1, "write_input 은 write 1회 그대로: {got:?}");
        assert_eq!(got[0], b"partial".to_vec());
    }

    // ── ADR-0088: 실패 표면화 ──
    #[test]
    fn write_input_observed_surfaces_transport_error() {
        struct FailingTransport;
        impl AgentTransport for FailingTransport {
            fn start(&self, _core: Arc<OutputCore>) {}
            fn send_input(&self, _input: InputEvent) -> Result<(), PtyError> {
                Err(PtyError::WriteFailed("stdin closed".into()))
            }
            fn resize(&self, _c: u16, _r: u16) -> Result<(), PtyError> {
                Ok(())
            }
            fn interrupt(&self) -> Result<(), PtyError> {
                Ok(())
            }
            fn shutdown(&self) {}
            fn capabilities(&self) -> crate::agent::types::TransportCaps {
                use crate::agent::types::{ControlCaps, InputCaps, OutputCaps, TransportCaps};
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
                        resize: false,
                        interrupt: false,
                        cancel: false,
                        graceful_shutdown: false,
                    },
                }
            }
        }
        let id = uuid::Uuid::new_v4();
        let core = Arc::new(OutputCore::new(
            id,
            0,
            Arc::new(NoopStatusSink),
            crate::agent::output_core::TurnWiring::detached(),
        ));
        let shell_cmd = crate::agent::profile::AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec![],
        };
        let session = AgentSession::new(
            id,
            PathBuf::from("."),
            0,
            80,
            24,
            Arc::new(AtomicU8::new(0)),
            ShellBackend.capabilities(&shell_cmd),
            InputEncoder::Raw,
            true,
            core,
            Box::new(FailingTransport),
        );
        let err = session.write_input_observed(b"x");
        assert!(
            matches!(err, Err(PtyError::WriteFailed(_))),
            "send_input 실패는 Err 로 표면화돼야(성공으로 삼키지 않음): {err:?}"
        );
    }

    // ── ClaudeStreamJson 인코더(ADR-0044) ──
    #[test]
    fn write_input_json_mode_wraps_as_stream_json_line() {
        let (session, captured) = session_with(InputEncoder::ClaudeStreamJson);
        session.write_input(b"hello").unwrap();
        let got = captured.lock().unwrap();
        assert_eq!(got.len(), 1);
        let line = &got[0];
        assert_eq!(*line.last().unwrap(), b'\n', "라인 종단 \\n");
        let s = String::from_utf8(line.clone()).unwrap();
        assert!(s.contains("\"type\":\"user\""), "user 턴 스키마: {s}");
        assert!(s.contains("\"text\":\"hello\""), "text 보존: {s}");
    }

    // ── ADR-0044/0045: 입력-시점 유저 에코 ──────────
    #[test]
    fn write_input_json_mode_emits_input_time_user_echo() {
        let (session, _captured) = session_with(InputEncoder::ClaudeStreamJson);
        let seen = Arc::new(Mutex::new(Vec::new()));
        session.subscribe(Arc::new(EmitCapturingSink {
            id: uuid::Uuid::new_v4(),
            seen: seen.clone(),
        }));

        session.write_input("안녕 클로드".as_bytes()).unwrap();

        let got = seen.lock().unwrap();
        assert_eq!(
            *got,
            vec!["structured:user".to_string()],
            "json 모드 write_input 은 입력-시점 유저 에코 1건을 emit 해야 함"
        );
    }

    #[test]
    fn write_input_terminal_mode_does_not_emit_user_echo() {
        // Raw(터미널·shell)는 PTY 로컬 에코가 이미 있어 합성 에코를 emit 하면 중복 → 아무 것도 emit 안 함.
        let (session, _captured) = session_with(InputEncoder::Raw);
        let seen = Arc::new(Mutex::new(Vec::new()));
        session.subscribe(Arc::new(EmitCapturingSink {
            id: uuid::Uuid::new_v4(),
            seen: seen.clone(),
        }));

        session.write_input(b"echo hi\r\n").unwrap();

        assert!(
            seen.lock().unwrap().is_empty(),
            "터미널(Raw) 경로는 입력-시점 유저 에코를 emit 하지 않아야 함(PTY 에코 중복 방지)"
        );
    }

    // ── json 모드 세션 caps: StdioTransport ⊕ ClaudeBackend 합성 ──
    #[cfg(windows)]
    #[test]
    fn json_mode_session_caps_are_structured() {
        let id = uuid::Uuid::new_v4();
        let core = Arc::new(OutputCore::new(
            id,
            0,
            Arc::new(NoopStatusSink),
            crate::agent::output_core::TurnWiring::detached(),
        ));
        let spec = crate::agent::types::CommandSpec {
            program: "cmd.exe".into(),
            args: vec!["/c".into(), "echo probe".into()],
            env: vec![],
            cwd: PathBuf::from("."),
        };
        // json 모드 = structured 캐리어 → StdioTransport 에 structured=true 주입(조립점 매핑).
        let (transport, _pid) = StdioTransport::open(&spec, true, None).expect("open");
        // json 모드 command — backend 가 이걸 보고 caps 를 산출한다.
        let json_cmd = crate::agent::profile::AgentCommand::Claude {
            extra_args: vec![],
            output_format: crate::agent::profile::ClaudeOutputFormat::StreamJson,
        };
        let session = AgentSession::new(
            id,
            PathBuf::from("."),
            0,
            80,
            24,
            Arc::new(AtomicU8::new(0)),
            // json 모드도 backend는 여전히 ClaudeBackend(resume/model은 프로그램 소관, ADR-0030).
            ClaudeBackend.capabilities(&json_cmd),
            InputEncoder::ClaudeStreamJson,
            true,
            core,
            Box::new(transport),
        );
        let caps = session.capabilities();
        assert!(caps.output.structured, "json 세션 → 구조화 출력");
        assert!(!caps.output.terminal_bytes, "터미널 바이트 아님");
        assert!(!caps.control.resize, "resize 불가");
        assert!(!caps.control.interrupt, "interrupt 불가(MVP)");
        // ★ADR-0044 후속 완료★: json 모드도 --resume 지원(spike-verified, claude 2.1.170) → resume=true.
        //   build_spec 이 SpawnMode::Resume 에서 --resume 을 내고 통제-sid(ADR-0008)를 재사용하므로 sid
        //   충돌 없음.
        assert!(
            caps.session.resume,
            "json 모드 세션 → resume=true(--resume 지원, spike-verified)"
        );
        session.kill(Duration::from_secs(5));
    }
}
