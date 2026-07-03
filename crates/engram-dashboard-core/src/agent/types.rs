/// 에이전트 고유 식별자
pub type AgentId = uuid::Uuid;

/// 구독자 Sink 고유 식별자 — subscribe 반환값, unsubscribe에 사용
pub type SinkId = uuid::Uuid;

/// 에이전트 생명주기 상태 — internally-tagged로 프론트에 discriminated union 전달
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
pub enum AgentStatus {
    Running,
    Exiting,
    Exited { code: Option<i32> },
    Failed { message: String },
    Killed,
}

/// pump→core 내부 출력 이벤트. 확장 가능 enum. core는 variant-agnostic(_ => ignore).
///
/// ★ADR-0045 (출력 정제를 백엔드로)★: 콘솔은 `TerminalBytes`(VT 바이트 스트림) 그대로,
/// 구조화 백엔드(claude stream-json 등)는 backend decoder가 파싱해 아래 구조화 variant로 emit한다.
/// 이 타입은 **core 도메인 타입**이지 protocol wire 타입이 아니다 — core↔wire 변환은 daemon
/// adapter가 한다(ADR-0003 격리: core는 wire를 모른다). core에 tauri import 금지(serde는 허용).
///
/// `turn_id`/`message_id`는 대화 추적용 optional 필드다 — claude는 안 채워도 되고, codex/gemini의
/// turn·message 모델 누수를 흡수하려 열어 둔다(교체성). backend가 못 채우면 None.
/// `Structured{kind,json}`은 위 정형 variant로 안 잡히는 backend별 이벤트의 탈출구다.
#[derive(Debug, Clone)]
pub enum OutputEvent {
    /// 콘솔 raw 바이트(VT 스트림). PtyTransport·터미널 모드의 유일 payload.
    TerminalBytes(Vec<u8>),
    /// 어시스턴트 텍스트 증분(스트리밍 델타).
    TextDelta {
        text: String,
        turn_id: Option<String>,
        message_id: Option<String>,
    },
    /// 도구 호출 — 이름 + 직렬화된 인자(JSON 문자열, backend별 스키마 그대로).
    ToolCall {
        name: String,
        args_json: String,
        /// 호출 식별자(권한 UX·결과 매칭용). claude tool_use id 등.
        id: Option<String>,
        turn_id: Option<String>,
        message_id: Option<String>,
    },
    /// 토큰 사용량.
    Usage {
        input_tokens: u64,
        output_tokens: u64,
        turn_id: Option<String>,
    },
    /// 한 메시지(turn 응답) 종료 신호.
    MessageDone {
        turn_id: Option<String>,
        message_id: Option<String>,
    },
    /// backend가 보고한 오류(스트림 내부 오류 등 — TerminalReason과 별개, 종료 아님).
    Error(String),
    /// 위 정형 variant로 안 잡히는 backend별 구조화 이벤트의 탈출구(forward-compat).
    /// kind=이벤트 종류 태그, json=원본 직렬화 payload. core는 내용을 해석하지 않는다.
    Structured { kind: String, json: String },
}

/// session→transport 입력 이벤트. 확장 가능 enum.
#[derive(Debug, Clone)]
pub enum InputEvent {
    Raw(Vec<u8>), // PTY 키 입력 바이트
                  // 후일: Message(String) / Reconfigure{..}
}

/// transport가 산출하는 종료 사유(flat). core가 AgentStatus로 매핑(finalize 1회).
/// ※ raw lib error(reqwest/nix) 직접 노출 금지 — 도메인 문자열로.
#[derive(Debug, Clone)]
pub enum TerminalReason {
    Exited { code: Option<i32> },
    Killed,
    Interrupted,
    StreamClosed,
    Cancelled,
    Error(String),
}

/// 유저 의도 — kill 핸들러가 채운다(ADR-0019). PTY 관측 사실(TerminalReason)과 **분리**한다:
/// 종료를 관측해 의도를 추론하면 데몬 셧다운 Job-kill 이 유저 kill 로 오분류되므로, 의도는
/// "종료를 일으킨 행동 지점"(kill 커맨드 핸들러)에서 명시적으로 태깅한다.
/// `#[repr(u8)]` — `Arc<AtomicU8>` 로 세션별 보관·snapshot 한다. DaemonShutdown 은 전역
/// `shutting_down` 플래그로 분리(여기 두지 않음).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminationIntent {
    None = 0,
    UserKill = 1,
}

impl TerminationIntent {
    /// AtomicU8 에 저장된 raw 값에서 복원. 알 수 없는 값은 보수적으로 None(=크래시 취급 경로).
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => TerminationIntent::UserKill,
            _ => TerminationIntent::None,
        }
    }
}

/// pump 가 finish 승자일 때 1회 발행하는 종료 이벤트(ADR-0019 reaper). reaper 한 스레드가
/// 소비해 sessions 맵 제거 + 프로필 disposition + 통지를 수행한다.
///
/// ★race 방지 핵심★: `intent_at_finish`/`shutting_down_at_finish` 는 **finish 그 순간** snapshot
/// 한 frozen 값이다. reaper 가 reap 시점에 live 로 읽으면 "크래시로 죽은 뒤 reaper 처리 전 유저가
/// kill→크래시를 유저kill 로 오분류→프로필 삭제(데이터 손실)" race 가 생긴다(consult GPT 적출).
#[derive(Debug, Clone)]
pub struct ReapMsg {
    pub id: AgentId,
    /// stale done 이 재spawn 된 새 세션을 오삭제 못 하게 reap 전 epoch 일치 검증(ADR-0007).
    pub epoch: u32,
    pub reason: TerminalReason,
    pub intent_at_finish: TerminationIntent,
    pub shutting_down_at_finish: bool,
}

/// 종료 분류 결과(ADR-0019 §decide). reap_one 이 lock 밖에서 ProfileRegistry 에 적용한다.
/// **downgrade-only**: auto_restore 를 true 로 절대 올리지 않는다(하드킬 안전망 성립 조건).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// 유저 kill·정상 /exit → 프로필 완전 삭제.
    DeleteProfile,
    /// 크래시·EOF·exit≠0·signal → 프로필 유지 + auto_restore=false(예약 복귀).
    KeepDisableAutoRestore,
    /// 데몬 셧다운 → 손 안 댐(auto_restore=true 잔류 → 부팅 복원).
    KeepAsIs,
}

/// transport에 주입하는 중립 실행 명세. backend가 산출. PtyTransport는 claude/codex를 모름.
#[derive(Debug, Clone)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: std::path::PathBuf,
}

/// 영역별 capability (bool 폭증 금지). 콘솔 값으로 채움. 직렬화(프론트 공유, snake_case).
///
/// ★출처 분리(load-bearing)★: 이 합성값의 5영역은 **두 출처**에서 온다 — input/output/control은
/// 물리 채널(transport)이, session/model은 프로그램(backend)이 결정한다. 예전엔 transport가
/// session.resume 까지 하드코딩해(claude·shell 무관 resume=true) shell 백엔드가 부정확했다.
/// 이제 `Capabilities::compose(TransportCaps, BackendCaps)`로만 만들어 출처를 타입으로 강제한다
/// (CLAUDE.md §2 capability 매트릭스: resize=transport-determined, resume/model=backend-determined).
#[derive(Debug, Clone, serde::Serialize)]
pub struct Capabilities {
    pub input: InputCaps,
    pub output: OutputCaps,
    pub control: ControlCaps,
    pub session: SessionCaps,
    pub model: ModelCaps,
}

/// 물리 채널(transport)이 **소유·결정**하는 caps. PTY/API 등 데이터 채널의 능력만 담는다.
/// transport는 session/model을 만들 수 없다(그 필드가 여기 없음 — 소유권을 타입으로 강제).
#[derive(Debug, Clone, serde::Serialize)]
pub struct TransportCaps {
    pub input: InputCaps,
    pub output: OutputCaps,
    pub control: ControlCaps,
}

/// 프로그램(backend: claude/shell/codex…)이 **소유·결정**하는 caps. resume 지원·모델 선택처럼
/// 채널이 아니라 실행 대상 프로그램의 능력만 담는다. backend는 input/output/control을 만들 수 없다
/// (그 필드가 여기 없음 — 소유권을 타입으로 강제).
#[derive(Debug, Clone, serde::Serialize)]
pub struct BackendCaps {
    pub session: SessionCaps,
    pub model: ModelCaps,
}

impl Capabilities {
    /// transport(물리)와 backend(프로그램) caps를 합쳐 최종 5영역 Capabilities를 만든다.
    /// 이게 Capabilities의 **유일한 정상 생성 경로**다 — 출처가 섞이지 않게(transport는 session을,
    /// backend는 control을 못 채우게) 타입으로 박았다.
    pub fn compose(t: TransportCaps, b: BackendCaps) -> Capabilities {
        Capabilities {
            input: t.input,
            output: t.output,
            control: t.control,
            session: b.session,
            model: b.model,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct InputCaps {
    pub raw: bool,
    pub message: bool,
    pub attachment: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct OutputCaps {
    pub terminal_bytes: bool,
    /// 출력이 구조화 스트림(NDJSON 등)이라 터미널 렌더가 아닌 파싱 렌더(RichSlot)가 필요함을 신고(ADR-0044).
    /// ★출처(ADR-0030)★: output 은 transport 소유 영역이다 — StdioTransport(json 모드 캐리어)는 조립점
    /// 주입값(json=true), PtyTransport(터미널)는 false 로 정직 신고한다.
    /// ★현 배선 상태(FIX 6c)★: caps 기반 렌더러 분기(xterm vs RichSlot)는 **M2 예정이며 아직 미배선**이다
    /// — 이 필드를 "현재 프론트 렌더 분기의 유일 근거"로 오독하지 말 것. M0 스파이크는 viewStore.richSlots
    /// 오버레이로 슬롯을 가른다. 이 필드는 M2 에서 그 분기의 근거가 되도록 **의도된** 신호다(ADR-0002).
    /// 내용 해석 아님(통로 무정제 불변) — "이 바이트 스트림은 터미널이 아니다"라는 렌더 힌트일 뿐.
    pub structured: bool,
    pub markdown: bool,
    pub tool_events: bool,
    pub usage: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ControlCaps {
    pub resize: bool,
    pub interrupt: bool,
    pub cancel: bool,
    pub graceful_shutdown: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionCaps {
    pub resume: bool,
    pub snapshot: bool,
    pub cwd_env: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelCaps {
    pub select: bool,
    pub temperature: bool,
    pub max_tokens: bool,
}

/// drain 내부 전달용 raw PTY 출력 청크 — 바이너리 그대로 (UTF-8 쪼개짐 방지)
#[derive(Debug, Clone, serde::Serialize)]
pub struct OutputChunk {
    pub seq: u64,
    pub data: Vec<u8>,
}

/// 프론트로 나가는 PTY 출력 wire 포맷 — base64 인코딩으로 JSON 호환.
/// ※S12: 이건 **Embedded(Tauri JSON Channel) 전용** 표현. base64는 JSON Channel 제약이며
/// 코어 관심사가 아니다 — ChannelOutputSink가 OutputFrame(raw)을 받아 이걸로 인코딩한다.
///
/// ★epoch★: WS binary frame 헤더([tag][agentId][epoch][seq])와 동형으로 출력 frame 마다
/// 세션 epoch 을 싣는다(OutputFrame.epoch 그대로). 인코딩 시 frame.epoch 을 **버리면**
/// embedded 가 epoch 0 고정으로 흘러, SubscribeAck.current_epoch≥1(resume-fallback) 과
/// 불일치해 ProtocolClient epoch 가드(f.epoch !== st.epoch)가 출력을 전멸시킨다(Stage 3
/// BLOCKER 1). 따라서 frame.epoch 을 반드시 동봉해 WS 경로와 동형화한다.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PtyEvent {
    pub agent_id: AgentId,
    pub seq: u64,
    pub epoch: u32,
    pub data_b64: String,
}

/// 코어→sink 출력 경계 (S12 raw 경계화). **raw 바이트를 빌려서** 전달 — base64/wire 인코딩은
/// sink 책임(Embedded=base64 PtyEvent, Daemon=binary frame). Copy(참조만)라 fanout 시 복사 0.
/// agent_id/epoch는 OutputCore가 보유한 불변값을 그대로 싣는다(데몬 frame 헤더용).
#[derive(Debug, Clone, Copy)]
pub struct OutputFrame<'a> {
    pub agent_id: AgentId,
    pub epoch: u32,
    pub seq: u64,
    pub data: &'a [u8],
}

/// 에이전트 메타데이터 스냅샷 — 목록 조회 및 상태 동기화용
#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentInfo {
    pub id: AgentId,
    /// 표시용 이름. ProfileRegistry(단일 진실원)에서 채운다. 프로필이 없으면 id 앞 8자.
    pub name: String,
    pub cwd: String,
    pub status: AgentStatus,
    pub cols: u16,
    pub rows: u16,
    /// 재spawn마다 +1. 프론트가 `[agentId, epoch]`로 재구독하는 트리거(S9 §18-a).
    pub epoch: u32,
    /// transport 종류별 지원 영역 — 프론트가 UI 분기에 사용.
    pub capabilities: Capabilities,
}

/// PTY 백엔드 오류 타입
#[derive(Debug, thiserror::Error)]
pub enum PtyError {
    #[error("agent not found: {0}")]
    NotFound(AgentId),
    #[error("spawn failed: {0}")]
    SpawnFailed(String),
    #[error("write failed: {0}")]
    WriteFailed(String),
    #[error("cwd outside workspace")]
    CwdDenied,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// transport가 해당 동작을 지원하지 않음(ApiTransport 껍데기 등). 동사별 미지원 신호.
    #[error("unsupported: {0}")]
    Unsupported(String),
}

/// 구독 replay 분기 결과(코어 중립 — 데몬이 protocol::SubscribeAction 으로 매핑).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayKind {
    /// 처음(oldest)부터 전체 replay — 신규 구독 또는 epoch 불일치.
    FromOldest,
    /// after_seq 가 ring oldest 보다 과거 → oldest 부터(앞부분 손실).
    Truncated,
    /// after_seq+1 부터 무손실 이어받기(tail 만).
    Resumed,
}

/// subscribe_from 결과 메타(데몬이 SubscribeAck 구성에 사용).
#[derive(Debug, Clone, Copy)]
pub struct SubscribeOutcome {
    pub kind: ReplayKind,
    pub sink_id: SinkId,
    pub oldest_seq: u64,
    pub latest_seq: u64,
    /// 실제 처음 전송한 chunk 의 seq. 보낼 게 없으면 "다음 live seq" 추정치.
    pub replay_from: u64,
    /// 실제 전송한 chunk 수(0 가능).
    pub replayed: usize,
}

/// OutputSink 전송 실패 신호 — drain이 감지 시 해당 구독자 제거 트리거
#[derive(Debug)]
pub struct SinkError;

/// PTY 출력 전달 추상화 — Tauri 의존 없이 headless 테스트 가능하게 격리.
/// ※S12: send는 **raw OutputFrame**을 받는다(base64 아님). wire 인코딩은 구현체가 소유:
/// ChannelOutputSink=base64 PtyEvent / WsOutputSink=binary frame. → 코어 transport-agnostic.
pub trait OutputSink: Send + Sync + 'static {
    fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError>;
    fn sink_id(&self) -> SinkId;
}

/// 에이전트 상태 변경 알림 추상화 — pty/가 AppHandle 없이 상위 층에 통보
pub trait StatusSink: Send + Sync + 'static {
    /// epoch 동봉(S9 §18-d): 프론트가 재spawn 후 옛 세션의 지연된 terminal 알림을
    /// epoch 불일치로 버릴 수 있게 한다(stale Killed 방어, fable C-1/Mn-1).
    fn status_changed(&self, id: AgentId, status: AgentStatus, epoch: u32);
    fn agent_list_updated(&self, agents: Vec<AgentInfo>);
    /// 복원 시도 결과 통지(S9 §18-d). 기본 no-op — 복원을 안 쓰는 sink는 구현 불필요.
    fn restore_result(&self, _report: crate::agent::profile::RestoreReport) {}
}

// ReplayBuffer 는 session.rs 로 이동 (LLD §1/§4: session.rs 소속).

#[cfg(test)]
mod tests {
    use super::*;

    // ── Capabilities::compose — 출처가 올바른 영역으로 합쳐지는지(소유권 합성 검증) ──
    // transport가 control.resize 를, backend가 session.resume 을 각각 결정하고, compose 가
    // 둘을 섞지 않고 제자리에 합치는지 단언한다. (이전 부정확: transport가 resume 까지 소유.)
    #[test]
    fn compose_merges_each_source_into_its_region() {
        // transport: 물리 채널만(control.resize=true, session/model 필드 없음).
        let t = TransportCaps {
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
        };
        // backend: 프로그램만(session.resume=true, model 전부 false).
        let b = BackendCaps {
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
        };

        let caps = Capabilities::compose(t, b);

        // 핵심: control.resize(transport 소유) ∧ session.resume(backend 소유)이 모두 살아 합쳐짐.
        assert!(
            caps.control.resize,
            "resize 는 transport 가 결정 → 합성에 보존"
        );
        assert!(
            caps.session.resume,
            "resume 은 backend 가 결정 → 합성에 보존"
        );
        // 출처가 뒤섞이지 않았는지 나머지도 확인.
        assert!(caps.input.raw);
        assert!(caps.output.terminal_bytes);
        assert!(caps.session.cwd_env);
        assert!(!caps.model.select);
    }
}
