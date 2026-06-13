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
#[derive(Debug, Clone)]
pub enum OutputEvent {
    TerminalBytes(Vec<u8>), // 콘솔 — 지금 유일 variant
                            // 후일: TextDelta(String) / MessageDone / Usage{..} / ToolCall{..} / Error(String)
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

/// transport에 주입하는 중립 실행 명세. backend가 산출. PtyTransport는 claude/codex를 모름.
#[derive(Debug, Clone)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: std::path::PathBuf,
}

/// 영역별 capability (bool 폭증 금지). 콘솔 값으로 채움. 직렬화(프론트 공유, snake_case).
#[derive(Debug, Clone, serde::Serialize)]
pub struct Capabilities {
    pub input: InputCaps,
    pub output: OutputCaps,
    pub control: ControlCaps,
    pub session: SessionCaps,
    pub model: ModelCaps,
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

/// 프론트로 나가는 PTY 출력 wire 포맷 — base64 인코딩으로 JSON 호환
#[derive(Debug, Clone, serde::Serialize)]
pub struct PtyEvent {
    pub agent_id: AgentId,
    pub seq: u64,
    pub data_b64: String,
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

/// OutputSink 전송 실패 신호 — drain이 감지 시 해당 구독자 제거 트리거
#[derive(Debug)]
pub struct SinkError;

/// PTY 출력 전달 추상화 — Tauri 의존 없이 headless 테스트 가능하게 격리
pub trait OutputSink: Send + Sync + 'static {
    fn send(&self, event: PtyEvent) -> Result<(), SinkError>;
    fn sink_id(&self) -> SinkId;
}

/// 에이전트 상태 변경 알림 추상화 — pty/가 AppHandle 없이 상위 층에 통보
pub trait StatusSink: Send + Sync + 'static {
    /// epoch 동봉(S9 §18-d): 프론트가 재spawn 후 옛 세션의 지연된 terminal 알림을
    /// epoch 불일치로 버릴 수 있게 한다(stale Killed 방어, fable C-1/Mn-1).
    fn status_changed(&self, id: AgentId, status: AgentStatus, epoch: u32);
    fn agent_list_updated(&self, agents: Vec<AgentInfo>);
    /// 복원 시도 결과 통지(S9 §18-d). 기본 no-op — 복원을 안 쓰는 sink는 구현 불필요.
    fn restore_result(&self, _report: crate::pty::profile::RestoreReport) {}
}

// ReplayBuffer 는 session.rs 로 이동 (LLD §1/§4: session.rs 소속).
