//! AgentSession — 에이전트 1개 = OutputCore(출력 측) + Box<dyn AgentTransport>(채널/자원 측) 합성.
//!
//! transport 종류(PTY/API)와 무관한 공용 표면을 노출하고, 내부에서 core/transport로 위임한다.
//!
//! 소유권 분할(impl-spec 표): AgentSession은 id/cwd/epoch/cols/rows + core(Arc) + transport(Box)만 든다.
//!   - master/child/shutdown/job/reader/writer → transport(PtyTransport) 안.
//!   - subscribers/replay/seq/status/finalized → core(OutputCore) 안.
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

/// 에이전트 1개 = 출력 측(core) + 채널/자원 측(transport)의 합성. transport 종류(PTY/API)와
/// 무관한 공용 표면을 노출하고, 내부에서 core/transport로 위임한다.
pub struct AgentSession {
    pub id: AgentId,
    pub cwd: PathBuf,
    pub epoch: u32,
    /// 현 터미널 폭/높이. resize 성공 시에만 갱신(실패 시 옛 값 유지) — manager.agent_info가 직접 load.
    pub cols: AtomicU16,
    pub rows: AtomicU16,
    /// 유저 종료 의도(ADR-0019). kill_agent 가 shutdown **전에** UserKill 로 태깅하고, finalize
    /// hook(spawn_session 이 이 Arc 를 캡처)이 finish 순간 snapshot 해 ReapMsg 에 싣는다.
    /// 세션별 신규 atomic. `Arc` 인 이유: hook 클로저가 같은 값을 공유 캡처한다.
    intent: Arc<AtomicU8>,
    /// backend(프로그램)가 결정한 caps(session/model). transport caps(input/output/control)와
    /// 합성해 최종 Capabilities 를 만든다 — capabilities()가 `Capabilities::compose` 로 매번 합성.
    /// manager.spawn 이 profile.command 로 산출해 주입한다(transport 는 이 값을 모른다).
    backend_caps: BackendCaps,
    /// write_input 을 transport 로 넘기기 **직전** 적용하는 입력 인코딩(ADR-0044/0004).
    /// 터미널·shell = Raw(바이트 무변환, 기존 동작 불변). json 모드 claude = ClaudeStreamJson
    /// (텍스트 턴을 claude 유저 JSON 라인으로 감쌈 — 스키마는 backend/claude.rs 단독 소유).
    /// session 은 이 태그만 들고 실제 스키마를 모른다(격리). manager.spawn 이 산출해 주입한다.
    encoder: InputEncoder,
    core: Arc<OutputCore>,
    transport: Box<dyn AgentTransport>,
}

impl AgentSession {
    /// 합성 세션 생성. **start는 여기서 호출하지 않는다** — manager가 new 이전에
    /// `transport.start(core.clone())`를 직접 부른다(impl-spec: 테스트 가시성·spawn 흐름 명시성).
    /// 즉 이 생성자는 이미 start된 transport와 core를 받아 묶기만 한다.
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
            core,
            transport,
        }
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
    /// ★배선 지점(ADR-0044)★: 여기서 encoder를 적용해 텍스트 턴을 백엔드 규약대로 감싼 뒤
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
    ///   `Err` 로 반환하지 `Ok` 로 축소 반환하지 않는다. `WriteOutcome.bytes_written` 은 written 카운트를
    ///   transport 밖으로 스레드해 얻은 독립 측정이 아니라 `bytes_requested` 의 by-construction 복사값이다
    ///   (완결성 판정 레버 아님 — `WriteOutcome` 주석). 완결성은 이 함수의 `Ok`/`Err` 로 본다.
    pub fn write_input_observed(&self, bytes: &[u8]) -> Result<WriteOutcome, PtyError> {
        // ★이 유저 턴의 메시지 uuid(replay dedup 키)★: 한 write_input 당 하나 생성해 (a) stdin user
        //   라인(encode)과 (b) 입력-시점 합성 에코(input_echo_event) **양쪽에 같은 값**으로 넘긴다.
        //   json 모드에서 claude 가 replay 로 이 uuid 를 그대로 되울리므로(실측), 프론트가 합성 에코와
        //   replay 를 uuid 로 합쳐 하나만 남긴다(중복 제거). session 은 불투명 Uuid 토큰만 알고 json
        //   형태·uuid 부착 위치는 모른다(ADR-0004 격리 — 스키마 지식은 backend/claude.rs 단독).
        //   Raw(터미널) encoder 는 이 uuid 를 무시하므로 터미널 경로 바이트는 불변이다.
        // ADR-0088: 이 msg_uuid 를 WriteOutcome 으로 노출한다(ingress msg_id 와 상관 — 값·의미 불변, 노출만).
        let msg_uuid = uuid::Uuid::new_v4();
        let encoded = self.encoder.encode(bytes, msg_uuid);
        self.transport.send_input(InputEvent::Raw(encoded))?;

        // ★ADR-0044/0045 · 왜: 입력-시점 유저 에코★: 터미널(Raw)은 PTY 가 입력을 즉시 로컬 에코하지만,
        //   json(stream-json) 모드는 claude 가 `--replay-user-messages` 로 되울릴 때까지(왕복 지연)
        //   유저 메시지가 화면에 안 뜬다. 그래서 send_input **성공 후**, encoder 가 json 모드면 동일한
        //   유저 이벤트를 즉시 core.emit 해 터미널의 즉시 에코를 흉내낸다(체감 반응성). 이후 claude 가
        //   되울린 replay 중복은 프론트 accumulator 가 uuid 로 dedup 한다(같은 msg_uuid) — decoder 는
        //   억제하지 않고 uuid 를 실어 그대로 통과시킨다(blunt-suppress → uuid dedup 교체, backend/claude.rs).
        //   과거/비매칭 uuid 의 user text(resume 재개분)는 dedup 되지 않아 전부 보존된다(vanish 회귀 제거).
        //   encoder=Raw 면 None → 터미널 경로는 아무 것도 추가로 emit 하지 않아 기존 동작 불변.
        //   ★락 규율(ADR-0006)★: 새 락 없이 core.emit 재사용 — emit 이 replay/subscribers 락을 짧게만
        //   잡고 lock 밖 send 하는 규율을 그대로 탄다. send_input 성공 후 emit 이라 순서도 자연스럽다.
        if let Some(event) = self.encoder.input_echo_event(bytes, msg_uuid) {
            self.core.emit(event);
        }
        // send_input Ok = write_all 성공 = 요청 바이트 전량 수용(완결성은 이 Ok 자체가 증거, WriteOutcome
        //   주석). bytes_written 은 독립 측정이 아니라 bytes_requested 의 by-construction 복사다(transport
        //   가 written 카운트를 안 돌려주고, write_all 계약상 Ok 면 항등). 둘 다 논리 메시지 바이트 수(char 아님).
        let n = bytes.len();
        // ADR-0088: write 가 착지한 incarnation 의 epoch = 이 write 를 집행한 세션의 self.epoch.
        //   bytes_written 과 같은 by-construction 값(독립 측정 아님) — write 를 수행한 세션이 자기 epoch 을
        //   그대로 실는다. 배달 관측 레코드가 "어느 incarnation 이 받았나" 를 레코드만으로 답하게 하려는 것.
        Ok(WriteOutcome {
            bytes_requested: n,
            bytes_written: n,
            msg_uuid,
            epoch: self.epoch,
        })
    }

    /// 터미널 크기 변경. transport.resize 성공 후에만 cols/rows atomic 갱신(? 연산자로 실패 시 옛 값 유지).
    /// 현 manager.resize의 atomic 저장 책임이 여기로 이관.
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), PtyError> {
        self.transport.resize(cols, rows)?;
        self.cols.store(cols, Ordering::Relaxed);
        self.rows.store(rows, Ordering::Relaxed);
        Ok(())
    }

    /// 진행 중 작업만 중단(≠kill). PTY=0x03 주입. 프로세스는 살아 있다.
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

    /// 과도기 Exiting 전이 — kill 직전 manager가 먼저 호출(stage 6). core로 위임.
    /// terminal(이미 종료)이면 false. enter_exiting과 kill은 별개 동사다.
    pub fn enter_exiting(&self) -> bool {
        self.core.enter_exiting()
    }

    /// 최종 capability — transport(물리: input/output/control)와 backend(프로그램: session/model)
    /// 의 합성. 출처가 타입으로 분리돼 있어 transport 가 resume 을, backend 가 resize 를 섞어
    /// 채우는 사고가 구조적으로 불가능하다(예전 부정확 = transport 의 resume 하드코딩 제거).
    pub fn capabilities(&self) -> Capabilities {
        Capabilities::compose(self.transport.capabilities(), self.backend_caps.clone())
    }

    /// 구독자 등록 → core. SinkId 반환(unsubscribe용).
    pub fn subscribe(&self, sink: Arc<dyn OutputSink>) -> SinkId {
        self.core.subscribe(sink)
    }

    /// after_seq/epoch 기반 선택적 replay 구독 → core. SubscribeOutcome 반환.
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

    /// 구독 해제 → core.
    pub fn unsubscribe(&self, sink_id: SinkId) {
        self.core.unsubscribe(sink_id);
    }

    /// replay 스냅샷 → core. 늦게 붙는 창 초기 복원용.
    pub fn snapshot(&self) -> Vec<OutputChunk> {
        self.core.snapshot()
    }

    /// 현재 상태 → core.
    pub fn status(&self) -> AgentStatus {
        self.core.status()
    }

    /// 현 cols/rows 게터(pub atomic 직접 load도 가능 — manager.agent_info 편의).
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

    /// send_input에 도착한 바이트를 그대로 기록하는 mock transport — write_input의 인코딩
    /// 배선을 실 프로세스 없이 단언하기 위한 격리 하네스(ADR-0012).
    struct CapturingTransport {
        captured: Arc<Mutex<Vec<Vec<u8>>>>,
    }
    impl AgentTransport for CapturingTransport {
        fn start(&self, _core: Arc<OutputCore>) {}
        fn send_input(&self, input: InputEvent) -> Result<(), PtyError> {
            let InputEvent::Raw(bytes) = input;
            self.captured.lock().unwrap().push(bytes);
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

    /// core 로 emit 된 출력 이벤트를 (kind, is_event) 로 수집하는 mock OutputSink —
    /// write_input 의 입력-시점 유저 에코 emit(ADR-0044/0045)을 실 프로세스 없이 단언하기 위한 하네스.
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
            // 구조화 이벤트만 태그 문자열로 수집(Structured 는 "structured:<kind>", 그 외는 variant 명).
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
        let id = uuid::Uuid::new_v4();
        let core = Arc::new(OutputCore::new(id, 0, Arc::new(NoopStatusSink)));
        let captured = Arc::new(Mutex::new(Vec::new()));
        let transport = Box::new(CapturingTransport {
            captured: captured.clone(),
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
            core,
            transport,
        );
        (session, captured)
    }

    // ── Raw 인코더: write_input이 바이트를 무변환으로 넘긴다(터미널 경로 회귀 불변) ──
    #[test]
    fn write_input_raw_is_byte_identical() {
        let (session, captured) = session_with(InputEncoder::Raw);
        let input = b"echo hi\r\n\x1b[A\x03";
        session.write_input(input).unwrap();
        let got = captured.lock().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0], input.to_vec(), "Raw 는 바이트 동일이어야 함");
    }

    // ── ADR-0088: write_input_observed 가 배달-경계 계측(논리 메시지 바이트 + msg_uuid)을 반환한다 ──
    #[test]
    fn write_input_observed_surfaces_bytes_and_msg_uuid() {
        // Raw: 논리 메시지 바이트 = 넘긴 입력 길이(exact). msg_uuid 는 유효값. 완결성은 Ok 자체가 증거.
        let (session, captured) = session_with(InputEncoder::Raw);
        let input = b"hello-observed"; // 14바이트 ASCII.
        let outcome = session
            .write_input_observed(input)
            .expect("write_input_observed ok");
        // ★exact 카운트(FIX-5)★: 요청 바이트는 넘긴 입력 길이와 **정확히** 같아야 한다(off-by-one/계층 회귀 거름).
        assert_eq!(
            outcome.bytes_requested, 14,
            "요청 바이트 = 넘긴 입력의 정확 바이트 수"
        );
        assert_eq!(outcome.bytes_requested, input.len(), "요청 = 입력 len");
        // bytes_written 은 bytes_requested 의 by-construction 복사(독립 측정 아님 — WriteOutcome 주석).
        //   이 등식은 short-write 를 못 잡는다(항상 성립) — 완결성 증거가 아니라 by-construction 항등 확인일 뿐.
        assert_eq!(
            outcome.bytes_written, outcome.bytes_requested,
            "by-construction 항등(bytes_written = bytes_requested 복사) — short-write 탐지 아님"
        );
        assert!(
            !outcome.msg_uuid.is_nil(),
            "이 유저 턴의 msg_uuid 를 노출해야(상관 키)"
        );
        // ADR-0088: epoch 은 write 를 집행한 세션의 self.epoch by-construction 복사(여기선 0).
        assert_eq!(
            outcome.epoch, 0,
            "WriteOutcome.epoch = write 를 수행한 세션의 epoch(by-construction)"
        );
        // 바이트는 여전히 그대로 통과(계측판이 Raw 바이트 동일성을 깨지 않음).
        assert_eq!(captured.lock().unwrap()[0], input.to_vec());
    }

    // ── ADR-0088(FIX-5): 멀티바이트(UTF-8) 본체 — 요청 바이트는 **바이트 수**(char 수 아님)여야 한다 ──
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

    // ── ADR-0088: send_input 실패는 write_input_observed 에서 Err 로 표면화(성공으로 삼키지 않음) ──
    #[test]
    fn write_input_observed_surfaces_transport_error() {
        // send_input 이 항상 Err 를 내는 mock transport — write 실패가 Ok(축소)로 숨지 않고 Err 로 올라와야 한다.
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
        let core = Arc::new(OutputCore::new(id, 0, Arc::new(NoopStatusSink)));
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
            core,
            Box::new(FailingTransport),
        );
        let err = session.write_input_observed(b"x");
        assert!(
            matches!(err, Err(PtyError::WriteFailed(_))),
            "send_input 실패는 Err 로 표면화돼야(성공으로 삼키지 않음): {err:?}"
        );
    }

    // ── ClaudeStreamJson 인코더: write_input이 claude 유저 JSON 라인으로 감싼다(ADR-0044) ──
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

    // ── ADR-0044/0045: 입력-시점 유저 에코 — json 모드는 emit, 터미널(Raw)은 안 함 ──────────
    #[test]
    fn write_input_json_mode_emits_input_time_user_echo() {
        let (session, _captured) = session_with(InputEncoder::ClaudeStreamJson);
        let seen = Arc::new(Mutex::new(Vec::new()));
        session.subscribe(Arc::new(EmitCapturingSink {
            id: uuid::Uuid::new_v4(),
            seen: seen.clone(),
        }));

        session.write_input("안녕 클로드".as_bytes()).unwrap();

        // json 모드 → 입력 직후 Structured{kind:"user"} 1건이 core 로 emit 돼야 한다(즉시 에코).
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

    // ── json 모드 세션 caps: StdioTransport ⊕ ClaudeBackend 합성 → 구조화 출력 + resize/interrupt false ──
    #[cfg(windows)]
    #[test]
    fn json_mode_session_caps_are_structured() {
        let id = uuid::Uuid::new_v4();
        let core = Arc::new(OutputCore::new(id, 0, Arc::new(NoopStatusSink)));
        let spec = crate::agent::types::CommandSpec {
            program: "cmd.exe".into(),
            args: vec!["/c".into(), "echo probe".into()],
            env: vec![],
            cwd: PathBuf::from("."),
        };
        // json 모드 = structured 캐리어 → StdioTransport 에 structured=true 주입(조립점 매핑).
        let (transport, _pid) = StdioTransport::open(&spec, true, None).expect("open");
        // json 모드 command — backend 가 이걸 보고 caps(resume=true, ADR-0044 후속 완료)를 산출한다.
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
        //   충돌 없음. 옛 resume=false(fresh sid 강제) 가정은 폐기.
        assert!(
            caps.session.resume,
            "json 모드 세션 → resume=true(--resume 지원, spike-verified)"
        );
        session.kill(Duration::from_secs(5));
    }
}
