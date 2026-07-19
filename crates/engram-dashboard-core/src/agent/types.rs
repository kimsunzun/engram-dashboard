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

/// 종료 분류 결과(ADR-0019 §decide, ADR-0083 개정). reap_one 이 lock 밖에서 ProfileRegistry 에
/// 적용한다. **downgrade-only**: auto_restore 를 true 로 절대 올리지 않는다(하드킬 안전망 성립 조건).
///
/// ★ADR-0083: 자동 삭제 폐지★ — 옛 `DeleteProfile`(유저 kill·정상 exit → 프로필 완전 삭제)
///   variant 를 제거했다. reaper 는 어떤 종료에도 프로필을 자동 삭제하지 않으므로 삭제 처분을
///   산출하지 않는다. 프로필 삭제는 명시적 사용자 명령(AgentCommand::DeleteProfile /
///   Tauri delete_profile)이 ProfileRegistry::remove 를 직접 호출할 뿐, 이 enum 을 거치지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// 모든 런타임 종료(유저 kill·정상 exit·크래시·EOF·signal) → 프로필 유지 + auto_restore=false
    /// (시체 보존 — 재활성화 시 --resume 로 이어받음). ADR-0083 으로 유저 kill·정상 exit 도 이리로.
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

// ── ADR-0086: 제어 채널 입구(MCP) — core seam ──────────────────────────────────────
//
// ★왜 core 에 추상 descriptor + seam 을 두는가★: 스폰되는 에이전트가 데몬의 제어 채널(MCP 입구)에
//   붙으려면 (a) 데몬이 (AgentId,epoch)별 토큰을 발급하고 (b) 그 토큰+엔드포인트를 backend 명령줄에
//   주입해야 한다. 그러나 **토큰 발급·MCP 서버·mcp-config 파일**은 전부 데몬 관심사(rmcp/axum/HTTP)라
//   core 에 들어오면 tauri-import-0 격리와 같은 정신(전송·인프라 무의존)이 깨진다. 그래서 OutputSink/
//   StatusSink 와 **동일한 idiom(ADR-0003)** 으로, core 는 순수 trait(`ControlChannel`) + 추상
//   descriptor(`ControlEndpoint`)만 알고 실제 구현은 데몬(`DaemonControlChannel`)이 준다.
//
// ★인과 airtight(ADR-0086 토큰 수명 = (AgentId,epoch))★: provision 은 spawn 경로(spec 조립 직전)에서,
//   revoke 는 **reaper 단일 소비자**(ADR-0019 — 모든 terminal 이 수렴하는 유일 지점) + kill_agent 에서
//   부른다. 그래서 epoch 회전(재활성화 bump)마다 새 토큰, 크래시/kill/EOF 어떤 terminal 이든 정확히
//   1회 revoke 가 보장된다(reaper 가 epoch 검증 후 remove 하는 그 자리에서 revoke).

/// 데몬이 발급하는 제어 채널 엔드포인트(추상 descriptor). backend 가 이걸 받아 자기 프로그램의
/// 방식으로 명령줄/env 에 주입한다(claude = `--mcp-config <path>` — 그 지식은 backend/claude.rs 단독,
/// ADR-0004). core/transport 는 url/token/path 문자열만 나르고 "MCP" 나 claude 플래그를 모른다.
#[derive(Debug, Clone)]
pub struct ControlEndpoint {
    /// 데몬 MCP Streamable HTTP 엔드포인트 URL(예: `http://127.0.0.1:<port>/mcp`).
    pub url: String,
    /// 이 (AgentId,epoch) 전용 bearer 토큰(HTTP Authorization 헤더에 실린다).
    /// ★보안★: 이 값은 로그에 찍지 않는다(mcp-config 파일에만 기록 — 파일은 revoke 시 삭제).
    pub token: String,
    /// backend 가 생성한 에이전트별 mcp-config 파일 경로(데몬이 만들고 revoke 시 지운다).
    /// backend/claude.rs 가 이 파일에 url+token 을 써서 `--mcp-config` 로 주입한다.
    pub config_path: std::path::PathBuf,
    /// ADR-0086 스텝 2(CLI 입구): 데몬이 위치를 찾아낸 `engram-send` CLI 바이너리 절대경로(있으면).
    /// 데몬 exe 의 형제라 배포 시 동거하나, 부분 빌드 등으로 없을 수 있다 → `None` 이면 backend 가
    /// 그 env(claude=`ENGRAM_SEND_EXE`)를 주입하지 않는다(token/url 은 그래도 주입 — CLI 만 못 씀).
    /// core 는 이 값을 해석하지 않고 문자열 경로만 나른다(형제 exe 탐색 지식은 데몬 소유 — lib.rs).
    pub send_exe: Option<std::path::PathBuf>,
}

/// 제어 채널 provision 실패 사유(ADR-0086 fail-closed). 파일 write·CSPRNG 실패 등 "제어 채널을 붙일
/// **의도가 있었으나 실패**"한 경우다 — spawn 이 이 Err 를 만나면 fail-closed 로 스폰을 중단한다(제어
/// 채널 없이 도는 에이전트를 만들지 않는다). ★absence 와 구분★: Ok(None)=제어 채널을 안 쓰는 정당한
/// 부재(Noop·shell), Err=쓰려다 실패(치명). core 는 문자열만 나른다(rmcp/io 타입 누수 방지, ADR-0003).
#[derive(Debug)]
pub struct ProvisionError(pub String);

impl std::fmt::Display for ProvisionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "control channel provision failed: {}", self.0)
    }
}

impl std::error::Error for ProvisionError {}

/// 제어 채널 provisioning seam(ADR-0086). 구현은 데몬(`DaemonControlChannel`)이, core 는 이 trait 만
/// 안다(OutputSink/StatusSink 와 동형 — ADR-0003 격리). 기본 구현체 = `NoopControlChannel`(제어 채널
/// 없는 경로·테스트용 — provision 이 Ok(None) 을 돌려 backend 가 아무 것도 주입하지 않는다).
pub trait ControlChannel: Send + Sync + 'static {
    /// (AgentId,epoch)용 토큰을 발급하고 mcp-config 파일을 만들어 엔드포인트를 돌려준다. spawn 경로에서
    /// spec 조립 직전 호출. 반환 3-값(fail-closed 계약, ADR-0086):
    ///   - `Ok(Some(ep))` — 제어 채널 발급 성공(backend 가 주입).
    ///   - `Ok(None)`     — 제어 채널을 **안 쓰는 정당한 부재**(Noop·shell-only·미구성). 스폰 계속.
    ///   - `Err(_)`       — 제어 채널을 쓰려다 **실패**(CSPRNG/파일 write 오류). ★치명★ — 스폰은
    ///     이 Err 를 만나면 fail-closed 로 중단한다(제어 채널 없이 몰래 도는 에이전트 금지, health 위장 방지).
    fn provision(&self, id: AgentId, epoch: u32)
        -> Result<Option<ControlEndpoint>, ProvisionError>;

    /// (AgentId,epoch)의 토큰을 폐기하고 mcp-config 파일을 지운다. 어떤 terminal(kill·크래시·EOF·정상
    /// 종료)에서든 reaper 단일 소비자가 부른다 → 정확히 1회 revoke. epoch 를 함께 받아 stale terminal 이
    /// 재활성화(epoch bump)로 새로 붙은 산 토큰을 지우지 못하게 한다(ADR-0007/0084 epoch-guard 정신).
    fn revoke(&self, id: AgentId, epoch: u32);
}

/// 제어 채널을 안 쓰는 경로(headless 테스트·shell-only)용 no-op 구현. provision 은 항상 Ok(None)
/// (정당한 부재 — 실패가 아님), revoke 는 무동작. AgentManager 기본값 — 데몬만 실제
/// `DaemonControlChannel` 을 주입한다.
pub struct NoopControlChannel;

impl ControlChannel for NoopControlChannel {
    fn provision(
        &self,
        _id: AgentId,
        _epoch: u32,
    ) -> Result<Option<ControlEndpoint>, ProvisionError> {
        Ok(None)
    }
    fn revoke(&self, _id: AgentId, _epoch: u32) {}
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

/// 코어→sink 출력 payload (S15 B5 payload-generic). **빌려서** 전달 — 콘솔 raw 바이트든
/// 구조화 이벤트든 sink 가 wire 로 인코딩한다(코어는 wire 를 모른다, ADR-0003).
/// ★ADR-0002 (출력 종류 비가정)★: 출력을 터미널 바이트로 강제하지 않는다 — Bytes/Event 두 갈래로
/// 나눠 sink 가 종류별로 처리(Bytes→tag0 terminal frame, Event→tag1 structured frame, B7)한다.
/// 참조만 담아 Copy 유지(OutputFrame Copy 계약 보존) — Serialize 미부착(core 도메인 타입, ADR-0003).
#[derive(Debug, Clone, Copy)]
pub enum OutputPayload<'a> {
    /// 콘솔 raw 바이트(터미널·tag0 경로). PtyTransport·터미널 모드의 payload.
    Bytes(&'a [u8]),
    /// 구조화 이벤트(tag1 경로 — B7 이 인코딩). backend decoder 가 파싱한 OutputEvent.
    Event(&'a OutputEvent),
}

/// 코어→sink 출력 경계 (S12 raw 경계화 → S15 B5 payload-generic). **payload 를 빌려서** 전달 —
/// base64/wire 인코딩은 sink 책임(Embedded=base64 PtyEvent, Daemon=binary frame). Copy(참조만)라
/// fanout 시 복사 0. agent_id/epoch는 OutputCore가 보유한 불변값을 그대로 싣는다(데몬 frame 헤더용).
///
/// ★S15 B5★: `data: &[u8]` → `payload: OutputPayload<'a>` — 콘솔 바이트(Bytes)와 구조화 이벤트(Event)
/// 를 한 경계로 흘린다(ADR-0002 출력 종류 비가정). sink 가 종류별로 인코딩(Bytes→tag0, Event→tag1).
#[derive(Debug, Clone, Copy)]
pub struct OutputFrame<'a> {
    pub agent_id: AgentId,
    pub epoch: u32,
    pub seq: u64,
    pub payload: OutputPayload<'a>,
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

/// 입력 write 의 배달-경계 계측 산출물(ADR-0088 Stage 0).
///
/// ★왜 존재하나★: `write_input`/`write_stdin` 이 `Ok(())` 만 돌려주면 "전송 실패로 안 꽂힘" 과
///   "다 꽂혔는데 모델이 무시" 를 구별할 증거가 없다. 배달 정확성 하네스(ADR-0088)가 이 둘을
///   가르려면 write 경계에서 **완결성 신호**(전량 수용 vs 실패)와 이 유저 턴의 replay-dedup 키
///   (`msg_uuid`)를 관측 가능하게 올려야 한다. 이 값이 그 산출물이다(성공 경로에서만 반환).
///
/// ★완결성 신호 = Ok-vs-Err 이지 바이트 비교가 아니다(중요)★: transport 의 `send_input` 은
///   `write_all`(+`flush`)로 쓴다 — `write_all` 은 요청 바이트를 **전부** 쓰거나 `Err` 를 낸다
///   (부분 write 를 `Ok` 로 숨기지 않는다, std 계약). 따라서 "전량 수용됐나"의 유일한 증거는
///   `Ok(WriteOutcome)` **자체**(vs `Err`)다 — 아래 두 바이트 필드의 비교가 아니다. 진짜 written
///   바이트 수는 transport 밖으로 스레드되지 않으므로(write_all 계약상 불필요), `bytes_written` 은
///   독립 계측값이 아니라 `bytes_requested` 를 **구성상 그대로 복사**한 값이다(short-write 탐지 불가 —
///   비교하면 항상 같다, 동어반복). 이 필드가 있는 이유는 관측 레코드의 자기설명(로그·forensic 에서
///   "이만큼을 write 요청했고 write_all 이 Ok 였다"는 by-construction 항등)일 뿐, 완결성 판정 레버가
///   아니다. 완결성은 `Ok` 를 봐야 한다.
///
/// ★바이트 단위·계층★: 여기 두 값은 **호출자가 세션 경계에 넘긴 논리 메시지의 바이트 길이**
///   (`bytes.len()` — encoder 감싸기 **전**, char 수 아님)다. encoder 가 텍스트를 감싸면(json 모드)
///   실제 wire 바이트는 이보다 크지만, 그 encoded wire 카운트는 여기서 재지 않는다(이 계층의 논리
///   단위가 아님). daemon 레이어의 `DeliveryObservation` 도 같은 "논리 메시지 바이트" 의미를 쓴다
///   (거기선 그 논리 메시지 = `wrap_message` 로 만든 봉투 문자열의 바이트).
#[derive(Debug, Clone, Copy)]
pub struct WriteOutcome {
    /// 호출자가 세션 경계에 넘긴 논리 메시지 바이트 수(`bytes.len()` — encoder 감싸기 **전**, char 수 아님).
    pub bytes_requested: usize,
    /// `bytes_requested` 의 by-construction 복사값. ★독립 측정이 아니다★: write_all 계약상 written 카운트가
    /// transport 밖으로 나오지 않아, `Ok` 면 요청 = 수용이 항등으로 성립하므로 그대로 복사한다. 완결성은 이
    /// 값이 아니라 `Ok`(vs `Err`)로 판정한다(struct 주석 참조). 비교는 short-write 를 못 잡는다(항상 같음).
    pub bytes_written: usize,
    /// 이 유저 턴의 메시지 uuid(replay-dedup 키, session.write_input 이 생성 — LOAD-BEARING).
    /// 배달 하네스가 ingress 의 논리 msg_id 와 이 값을 상관시켜 "claude 가 이 턴을 replay 했나"(=
    /// 실제 파싱했나)를 판정한다(ADR-0088). 값·의미는 여기서 바꾸지 않는다 — 노출만 한다.
    pub msg_uuid: uuid::Uuid,
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
