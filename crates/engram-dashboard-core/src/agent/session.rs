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
    SinkId, SubscribeOutcome, TerminationIntent,
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
        let encoded = self.encoder.encode(bytes);
        self.transport.send_input(InputEvent::Raw(encoded))
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
        // json 모드 command — backend 가 이걸 보고 mode 별 caps(resume=false, FIX 5)를 산출한다.
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
        // ★FIX 5★: json 모드는 resume=false(fresh sid 강제) — 예전 true 신고는 sid 충돌 지뢰였다.
        assert!(
            !caps.session.resume,
            "json 모드 세션 → resume=false(ADR-0044 후속, sid fresh 강제)"
        );
        session.kill(Duration::from_secs(5));
    }
}
