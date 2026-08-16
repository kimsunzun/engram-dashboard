//! wire 메시지 — UI→core [`AgentCommand`], core→UI [`AgentEvent`].
//! 둘 다 externally-tagged JSON(serde 기본).

use engram_dashboard_command::{CommandDecl, OwnerToken};
use ts_rs::TS;

use crate::domain::{
    AgentInfo, AgentProfile, AgentStatus, Capabilities, ClaudeOutputFormat, EnvelopeFormat, Preset,
    RestoreReport, SnapshotChunk,
};
use crate::ids::{AgentId, PresetId, ProfileId, RequestId};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
// ★`Auth` variant 는 여기 없다(ADR-0129 0-4, 2026-08-05)★: 연결 후 첫 frame 전용 인증 프레임의 모양은
//   네트워크 lib(`engram-dashboard-net` 의 `auth::AuthFrame`)이 소유한다 — 토큰 인증은 "이 소켓을 살릴지"
//   판정이라 네트워크 살림이고, 그 판정을 하는 crate 가 에이전트 어휘(이 enum)를 타입으로 알면 안 되기
//   때문이다(ADR-0129 결정 1). **wire 는 그대로다** — 저쪽도 externally-tagged 라 프레임은 여전히
//   `{"Auth":{"token":"…","protocol_version":N}}` 이고, 그 형태의 정본 테스트는 저 crate 의 golden JSON 이다.
//   ★여기에 다시 넣지 말 것★: 두 정의가 공존하면 조용히 갈라진다. 프론트 바인딩(`bindings/AgentCommand.ts`)
//   에도 그래서 `Auth` 가 없다(프론트 `wsTransport` 는 원래 타입 없이 객체 리터럴로 만든다).
//
// ★이름 충돌★ — core `agent::profile::AgentCommand` 는 뜻이 다르다(프로필이 띄울 프로그램).
//   그쪽의 wire 미러는 이 enum 이 아니라 `AgentSpawnCommand` 다. crate 를 빼고
//   "AgentCommand" 라 부르면 뜻이 안 정해진다.
pub enum AgentCommand {
    Spawn {
        #[ts(type = "string")]
        profile_id: ProfileId,
        request_id: RequestId,
    },
    Kill {
        #[ts(type = "string")]
        agent_id: AgentId,
        request_id: RequestId,
    },
    /// 진행 중 작업만 중단(Ctrl+C). 프로세스는 생존.
    Interrupt {
        #[ts(type = "string")]
        agent_id: AgentId,
        request_id: RequestId,
    },
    WriteStdin {
        #[ts(type = "string")]
        agent_id: AgentId,
        #[serde(with = "serde_bytes")]
        #[ts(type = "number[]")]
        data: Vec<u8>,
        request_id: RequestId,
    },
    /// viewport_id 는 멀티뷰 중 어느 뷰가 요청했는지(ControlLease 판정용).
    Resize {
        #[ts(type = "string")]
        agent_id: AgentId,
        cols: u16,
        rows: u16,
        viewport_id: Option<String>,
    },
    /// epoch/after_seq 로 재연결 resume(설계 §1-3).
    /// 둘 다 None = 처음부터(oldest 부터) 받겠다는 신규 구독.
    Subscribe {
        #[ts(type = "string")]
        agent_id: AgentId,
        epoch: Option<u32>,
        #[ts(type = "number | null")]
        after_seq: Option<u64>,
    },
    Unsubscribe {
        #[ts(type = "string")]
        agent_id: AgentId,
    },
    /// 입력 lease 획득 요청(다중 뷰어 입력 충돌 방지, Zellij 명시 lease 모델). lease 가 비었으면
    /// 이 연결이 입력 권한을 잡는다. 이미 다른 연결이 보유하면 Error. §5: LLM 도 이 명령으로 권한을 쥔다.
    AcquireInput {
        #[ts(type = "string")]
        agent_id: AgentId,
        request_id: RequestId,
    },
    /// 보유자만 해제할 수 있다(보유자 아니면 Error). 해제 후엔 누구나 다시 acquire 가능.
    ReleaseInput {
        #[ts(type = "string")]
        agent_id: AgentId,
        request_id: RequestId,
    },
    /// 전체 에이전트 목록 조회(연결 직후 데몬이 자동 push 도 하지만 명시 조회도 허용).
    /// 응답은 request_id 동봉 [`AgentEvent::AgentList`](전용 reply).
    ListAgents { request_id: RequestId },
    /// 데몬 종료(§5 LLM 제어). force=true 면 활성 에이전트 있어도 종료, kill_agents=true 면 함께 정리.
    StopDaemon {
        force: bool,
        kill_agents: bool,
        request_id: RequestId,
    },

    // ── 프로필 CRUD + ad-hoc spawn(phase4 1단계) ───────────────────────────────────
    /// cwd 만으로 ad-hoc 셸 에이전트 spawn — 호출자가 미리 만들어 둔 프로필이 필요 없다. 기본 셸 명령
    /// 프로필을 즉석 생성(생성 시 auto_restore=false)해 Fresh spawn 하고, spawn 경로가 그 프로필을
    /// registry 에 등록·persist 한다.
    SpawnByCwd { cwd: String, request_id: RequestId },

    /// 응답은 request_id 동봉 [`AgentEvent::ProfileList`](전용 reply).
    ListProfiles { request_id: RequestId },

    /// claude 프로필 생성(스폰하지 않음 — 등록·persist만). ※env 에 자격증명 금지(평문 persist).
    CreateProfile {
        name: String,
        cwd: String,
        extra_args: Vec<String>,
        env: Vec<(String, String)>,
        auto_restore: bool,
        /// `#[serde(default)]` 라 이 필드 없는 옛 프론트/wire 는 Terminal 로 흡수(기존 동작 불변,
        /// PROTOCOL_VERSION 유지 — sibling OutputCaps.structured 와 같은 additive·tolerant 접근).
        #[serde(default)]
        output_format: ClaudeOutputFormat,
        request_id: RequestId,
    },

    DeleteProfile {
        #[ts(type = "string")]
        profile_id: ProfileId,
        request_id: RequestId,
    },

    /// resume=true 면 기존 세션 이어받기(claude `--resume`).
    SpawnProfile {
        #[ts(type = "string")]
        profile_id: ProfileId,
        resume: bool,
        request_id: RequestId,
    },

    SetProfileAutoRestore {
        #[ts(type = "string")]
        profile_id: ProfileId,
        auto_restore: bool,
        request_id: RequestId,
    },

    /// 프로필 표시명 override 설정/해제(ADR-0061 리치화 — 트리 rename). `name=Some` → override 저장,
    /// `None` → 해제(cwd basename 파생 복귀). ★정규화는 데몬 저장 게이트(`AgentManager::rename_agent`)
    /// 책임★ — 양끝 공백 제거와 "공백만 남으면 override 없음" 판정을 이름 유일성 판정 **전에** 거기서
    /// 끝낸다. 그래서 `" bob "` 이 그대로 와도 `bob` 요청으로 확정되고, 같은 요청 재제출도 게이트가 멱등
    /// 처리한다(접미사 번호 미소모). 프론트가 미리 다듬어 보내도 되지만 그건 UX 편의지 계약이 아니다.
    /// 없는 id 면 Error(SetProfileAutoRestore 와 동형). 성공 후 [`AgentEvent::ProfileListUpdated`]
    /// broadcast(낙관 갱신 X — 모든 창 동기화).
    RenameProfile {
        #[ts(type = "string")]
        profile_id: ProfileId,
        #[ts(type = "string | null")]
        name: Option<String>,
        request_id: RequestId,
    },

    /// 트리 부모 지정/해제(ADR-0072 계층 reparent). `parent_id=Some(pid)` → child 를 pid 의 자식으로
    /// (1단 중첩), `None` → 루트 승격. 검증(self-parent·nonexistent parent·1단 상한·2단 금지)은 데몬이
    /// `ProfileRegistry::reparent` 로 한 임계구역에서 수행 — 위반이면 Error, 성공이면 Ack +
    /// [`AgentEvent::ProfileListUpdated`] broadcast(RenameProfile 와 동형 — 모든 창 동기화, 낙관 갱신 X).
    /// §5로 LLM/사용자가 같은 command 로 트리를 구성한다(사람 드래그는 보조 입력).
    ReparentProfile {
        #[ts(type = "string")]
        child_id: ProfileId,
        #[ts(type = "string | null")]
        parent_id: Option<ProfileId>,
        request_id: RequestId,
    },

    /// 응답은 [`AgentEvent::Snapshot`].
    /// (Subscribe replay 와 별개의 1회성 조회.)
    GetSnapshot {
        #[ts(type = "string")]
        agent_id: AgentId,
        request_id: RequestId,
    },

    // ── 프리셋 CRUD(ADR-0061) ──────────────────────────────────────────────────────
    // 프리셋 = 스폰 전 "cwd 북마크"(인스턴스 아님). 데몬이 presets.json 을 단일 소유하고
    // wire 로만 CRUD 한다.
    /// 응답은 request_id 동봉 전용 reply [`AgentEvent::PresetList`](요청 연결에만).
    ListPresets { request_id: RequestId },

    /// 프리셋 생성(등록·persist만 — 스폰하지 않음). cwd 는 데몬이 정규화(dunce::canonicalize)해 저장.
    /// 이름은 저장 안 함(cwd basename 파생 — ADR-0061). 성공 후 [`AgentEvent::PresetListUpdated`] broadcast.
    CreatePreset { cwd: String, request_id: RequestId },

    /// 프리셋 삭제(등록 해제·persist). ★프리셋 삭제 ≠ 에이전트 종료★(ADR-0061) — 그 프리셋으로 이미
    /// 스폰된 에이전트는 무관하게 산다. 없는 id 면 no-op. 성공 후 [`AgentEvent::PresetListUpdated`] broadcast.
    DeletePreset {
        #[ts(type = "string")]
        preset_id: PresetId,
        request_id: RequestId,
    },

    /// 프리셋 표시명 override 설정/해제(ADR-0061 리치화). `name=Some` → override 저장, `None` → 해제
    /// (cwd basename 파생 복귀). trim·빈문자열 거부·미변경 스킵은 프론트가 확정 직전 처리 — 여기엔 유효 값
    /// 또는 명시 None 만 온다. 없는 id 면 no-op(DeletePreset 과 동형 Ack). 성공 후 [`AgentEvent::PresetListUpdated`]
    /// broadcast(낙관 갱신 X — 모든 창 동기화, ADR-0061 불변식).
    RenamePreset {
        #[ts(type = "string")]
        preset_id: PresetId,
        #[ts(type = "string | null")]
        name: Option<String>,
        request_id: RequestId,
    },

    /// 봉투 포맷 전역 스위치(ADR-0096) — A→B 메시지 봉투를 colon/xml 로 전환한다. 데몬이 **전역 상태
    /// 하나**를 들고, 이후 모든 `wrap_message` 조립이 그 값을 읽는다(ADR-0086 단일 wrap point
    /// 불변 유지 — 상태는 입력일 뿐 조립은 여전히 한 곳). ★조종 표면 전용★: 이 커맨드는 src-tauri Tauri
    /// command `set_envelope_format` 가 데몬으로 전달하는 경로로만 온다 — 워커 MCP 채널엔 노출하지 않는다
    /// (관리당하는 에이전트가 팀-전역 포맷을 바꾸면 안 됨, ADR-0096 결정 3·ADR-0094 최소권한).
    /// 지속성 없음(데몬 재시작 시 리셋 — 백로그).
    SetEnvelopeFormat {
        format: EnvelopeFormat,
        request_id: RequestId,
    },

    // ── 명령 버스 등록 wire(ADR-0134/0135 · TRD §3-7) ───────────────────────────────
    // ★받는 쪽만 서 있다★ — 데몬 dispatch 가 이 셋을 주인 명부에 반영하고 [`AgentEvent::CommandList`]
    //   로 답한다(`connection_core.rs`). **보내는 쪽은 아직 없다** — 셸·화면이 자기 선언을 얹는 것은
    //   TRD §6 Step 3·4 다.
    //
    // ★`request_id` 에 `#[serde(default)]` 를 달지 말 것★ — 이 crate 의 [`RequestId::default`] 는
    //   **새 v4 를 찍는다**(`ids.rs`). 기본값을 허용하면 그 칸이 빠진 패킷이 거절 대신 **아무와도 짝이
    //   안 맞는 새 상관 키**를 얻어 답장이 영영 안 붙는다(도구 crate 쪽 동명 타입은 정반대로 `Default`
    //   자체가 없다 — `engram-dashboard-command` 의 `envelope.rs` 가 그 이유를 적고 있다).
    /// 주인이 **붙는 순간** 자기 선언 전량을 한 방에 얹는다(TRD §3-7 조항 1). 이름마다 왕복하지 않고,
    /// 재연결마다 전량 재전송한다.
    ///
    /// ★`owner` 로 인수인계를 기대하지 말 것 — 이 칸은 **광고**다★: 데몬은 명부의 주인을 **그 패킷이 온
    /// 연결**에서 파생하고 이 칸은 쓰지 않는다(신원은 봉투가 아니라 연결이라는 계약 — TRD §4-⑧).
    /// 그래서 보내는 쪽이 고정된 `owner` 문자열을 만들어 두어도 그 값으로는 아무것도 이어받지 못한다.
    /// 인수인계는 **이름 단위 last-wins** 로 일어난다 — 재연결한 새 연결이 같은 이름을 다시 얹으면 그
    /// 이름의 주인이 새 연결로 넘어온다(TRD §3-7 조항 1·4).
    ///
    /// `decls` 의 `help` 는 **불투명 문자열**이다 — 받는 쪽(데몬)이 파싱·검증하거나 그 내용으로
    /// 분기하면 위반이다(TRD §3-7 하드 제약). 자료형을 `String` 위로 올리지 않는 것 자체가 그 게이트다.
    /// `catalog_version` 은 보낸 쪽 crate 의 세대 번호이고 **진단용**이다 — 받는 쪽이 자기 번호와
    /// 비교해 거절하면 틀린다(TRD §4-①).
    RegisterCommands {
        #[ts(type = "string")]
        owner: OwnerToken,
        #[ts(type = "Array<{ name: string, help: string }>")]
        decls: Vec<CommandDecl>,
        catalog_version: u32,
        request_id: RequestId,
    },

    /// 붙어 있는 동안의 **차분** — 늦게 뜬 기능이 이름을 더하고 꺼진 기능이 이름을 내린다
    /// (TRD §3-7 조항 3). 전량 재전송은 [`AgentCommand::RegisterCommands`] 뿐이다.
    ///
    /// `removed` 가 이름만 나르는 것은 의도다 — 내릴 때 모양은 필요 없다. 그리고 내린 이름은 명부에서
    /// **지워지지 않고 자취로 남는다**(ADR-0135) — 지우면 아직 붙어 있는 주인의 실재했던 이름이
    /// `UNKNOWN_COMMAND`(재시도 무의미)로 나간다.
    UpdateCommands {
        #[ts(type = "string")]
        owner: OwnerToken,
        #[ts(type = "Array<{ name: string, help: string }>")]
        added: Vec<CommandDecl>,
        removed: Vec<String>,
        request_id: RequestId,
    },

    /// 명부 전량 조회. 응답은 request_id 동봉 [`AgentEvent::CommandList`](전용 reply).
    ListCommands { request_id: RequestId },
}

/// 데몬 명부의 **클라이언트 투영** 한 줄([`AgentEvent::CommandList`] 의 원소).
///
/// ★버스 계약이 아니라 wire 계약이라 여기 산다★ — 도구 crate 의 `CommandDecl`(등록 단위)과 필드가
/// 겹치지만 방향과 주인이 다르다: `CommandDecl` 은 주인→데몬이 얹는 것이고, 이쪽은 데몬이 명부를
/// 훑어 내려 주는 것이라 명부만 아는 칸(`available`)이 하나 더 붙는다.
///
/// `available=false` = **이름은 명부에 있으나 주인이 지금 없다**(연결이 끊긴 자취). 없는 이름과 갈라야
/// 호출자가 재시도할지를 정할 수 있어서 지우지 않고 남긴다 — 전자는 `OWNER_UNAVAILABLE`(나중에 다시),
/// 후자는 `UNKNOWN_COMMAND`(재시도 무의미)다(TRD §4-②). 만료는 없다 — 시간이 지나 자취가 사라지면
/// 같은 질문의 답이 시계에 따라 갈려 그 구분 자체가 무너진다(ADR-0135).
/// `help` 는 주인이 얹은 문자열 **그대로**다(데몬이 열어보지 않으므로 가공도 없다).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct CommandListEntry {
    pub name: String,
    pub help: String,
    pub available: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub enum AgentEvent {
    /// 연결 직후 핸드셰이크.
    Hello {
        protocol_version: u32,
        daemon_version: String,
        /// 데몬 전체 capability(에이전트별 capability 는 AgentInfo 에).
        capabilities: Option<Capabilities>,
    },
    /// side-effect command 수신/처리 확인.
    Ack {
        request_id: RequestId,
    },
    SubscribeAck {
        #[ts(type = "string")]
        agent_id: AgentId,
        action: SubscribeAction,
        current_epoch: u32,
        #[ts(type = "number")]
        oldest_seq: u64,
        #[ts(type = "number")]
        latest_seq: u64,
        /// 이 seq+1 부터 replay 를 보낸다(클라가 dedup 기준).
        #[ts(type = "number")]
        replay_from: u64,
        /// ring 밖으로 밀려 일부 손실(clear+tail).
        truncated: bool,
    },
    /// 저빈도 구조화 출력(TextDelta/Usage/ToolCall 등).
    Output {
        #[ts(type = "string")]
        agent_id: AgentId,
        epoch: u32,
        #[ts(type = "number")]
        seq: u64,
        chunk: OutputChunk,
    },
    /// replay 구간 끝 — 이후는 라이브(C4 원자 전환의 클라측 신호).
    ReplayComplete {
        #[ts(type = "string")]
        agent_id: AgentId,
        epoch: u32,
    },
    /// epoch 동봉(옛 세션 stale 알림 방어).
    StatusChanged {
        #[ts(type = "string")]
        agent_id: AgentId,
        status: AgentStatus,
        epoch: u32,
    },
    /// 전체 목록 갱신(broadcast). terminal 판정은 이걸로(status_changed 아님 — 설계 불변식).
    AgentListUpdated {
        agents: Vec<AgentInfo>,
    },
    /// ListAgents 조회 응답(전용 reply) — request_id 에코로 "내 요청 결과"를 정확히 매칭.
    /// broadcast 인 AgentListUpdated 와 페이로드는 동일하나 편승 매칭(다음 도착 메시지 짝짓기)을
    /// 제거하기 위해 request_id 를 동봉한다(Spawned/Created 와 동형).
    AgentList {
        request_id: RequestId,
        agents: Vec<AgentInfo>,
    },
    RestoreResult {
        report: RestoreReport,
    },
    /// 입력 lease 상태 변경 통보(다중 뷰어가 "지금 잠겨있음"을 알게 함). held=true 면 누군가 보유 중,
    /// false 면 비어 있음(아무나 acquire 가능). 보유자 conn 식별값은 보안상 노출하지 않는다(잠김 여부만).
    InputLeaseChanged {
        #[ts(type = "string")]
        agent_id: AgentId,
        held: bool,
    },
    /// 프로필 목록 갱신(broadcast). CRUD(생성/삭제/토글) 후 자동 push.
    ProfileListUpdated {
        profiles: Vec<AgentProfile>,
    },
    /// ListProfiles 조회 응답(전용 reply) — request_id 에코. broadcast 인 ProfileListUpdated 와
    /// 페이로드는 같으나 편승 매칭 제거를 위해 request_id 동봉(Spawned/Created 와 동형).
    ProfileList {
        request_id: RequestId,
        profiles: Vec<AgentProfile>,
    },

    /// 프리셋 목록 갱신(broadcast, ADR-0061). CRUD(생성/삭제) 후 자동 push.
    PresetListUpdated {
        presets: Vec<Preset>,
    },
    /// ListPresets 조회 응답(전용 reply, ADR-0061) — request_id 에코. broadcast 인 PresetListUpdated 와
    /// 페이로드는 같으나 편승 매칭 제거를 위해 request_id 동봉(ProfileList 와 동형).
    PresetList {
        request_id: RequestId,
        presets: Vec<Preset>,
    },

    /// GetSnapshot 응답(전용 reply) — 그 시점 replay buffer 스냅샷.
    /// request_id 에코로 같은 agent 동시 조회를 정확히 매칭(이전 agent_id 편승 매칭 제거).
    /// broadcast 아님(특정 요청에만 응답).
    Snapshot {
        request_id: RequestId,
        #[ts(type = "string")]
        agent_id: AgentId,
        chunks: Vec<SnapshotChunk>,
    },

    /// CreateProfile 응답 — 생성된 프로필을 request_id 에 동봉(DaemonClient 가 "내 것" 매칭용).
    /// 기존 ProfileListUpdated broadcast 와 별개(그건 전 연결 미러 갱신용, request_id 없음).
    Created {
        request_id: RequestId,
        profile: AgentProfile,
    },
    /// SpawnByCwd/SpawnProfile 응답 — spawn 된 AgentInfo 를 request_id 에 동봉.
    /// 기존 AgentListUpdated broadcast 와 별개(StatusSink 가 전 연결에 push, request_id 없음).
    Spawned {
        request_id: RequestId,
        agent: AgentInfo,
    },

    /// [`AgentCommand::ListCommands`] 응답(전용 reply, ADR-0134/0135) — request_id 에코.
    /// broadcast 가 아니다(요청한 연결에만 간다). 데몬 dispatch 가 명부를 훑어 이걸 낸다.
    ///
    /// ★구형 셸 안전은 오직 "전용 reply" 라는 사실에만 기댄다★ — 구형 셸은 `ListCommands` 를 보내지
    /// 않으니 이 답장도 못 받고, 설령 받아도 모르는 externally-tagged variant 키는 조용히 버린다(에러도
    /// 로그도 없음). **이 variant 를 나중에 broadcast(요청 없이 push)로 바꾸는 순간** 모든 구형 셸이
    /// 그 이벤트를 아무 신호 없이 잃는다 — broadcast 로 바꾸려면 이 안전을 다시 설계해야 한다.
    ///
    /// 주인이 지금 없는 이름도 함께 실려 온다(`available=false`) — 걸러 내면 「이름은 아는데 주인이
    /// 없다」를 호출자가 볼 수 없어 §4-② 의 두 오류가 합쳐진다.
    CommandList {
        request_id: RequestId,
        entries: Vec<CommandListEntry>,
    },

    /// request_id 있으면 특정 command 실패.
    Error {
        request_id: Option<RequestId>,
        message: String,
    },
}

/// Subscribe 결과 분기(설계 §1-3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub enum SubscribeAction {
    /// epoch 불일치 → 완전 초기화 후 oldest 부터.
    Reset,
    /// epoch 일치 & after_seq<oldest → oldest 부터(앞부분 손실, clear+tail).
    TruncatedReplay,
    /// epoch 일치 & after_seq>=oldest → after_seq+1 부터 무손실 이어받기.
    Resume,
}

/// 구조화 출력 이벤트 wire 미러(ADR-0045 tag1 StructuredEvent) — core `OutputEvent`의 **충실한 미러**.
///
/// ★왜 새 타입인가(OutputChunk 확장 아님)★: 기존 wire `OutputChunk`(아래)는 S14 잔재라 `turn_id`/
/// `id`/`message_id`가 없고 `MessageDone`/`Error` variant도 없다. 게다가 `AgentEvent::Output`·
/// `export_all_to` 사용처에 묶여 있어 확장하면 그 계약이 깨질 위험이 있다. ADR-0045 "self-describing +
/// 교체성(optional turn_id/message_id 보존)"을 만족하려면 core `OutputEvent`를 필드 유실 0으로 미러해야
/// 하므로, 오염 없는 **새 wire 타입**을 신설한다(OutputChunk 는 GetSnapshot 스냅샷 전용으로 그대로 둔다).
///
/// ★core↔wire 변환은 daemon adapter★(ADR-0003 격리): core `OutputEvent`(도메인 타입, Serialize 미부착)
/// → 이 wire 타입은 daemon `connection_core::output_event_to_wire` 가 명시 매핑한다. protocol 은 wire
/// 타입만 소유(core 무의존).
///
/// ★TerminalBytes 는 제외★: 콘솔 raw 바이트는 tag0 terminal frame(payload=raw bytes)으로만 흐르고
/// tag1 payload 에 실리지 않는다. 따라서 이 미러에는 TerminalBytes variant 를 두지 않는다 — core
/// `OutputEvent::TerminalBytes` 가 이 변환에 오면 adapter 가 방어적으로 흡수(근거 주석은 output_event_to_wire).
///
/// ★self-describing serde★: internally-tagged(`#[serde(tag="type")]`) — payload JSON 에 `"type"` 판별자가
/// 박혀 프론트가 JSON.parse 후 variant 를 가른다.
/// wire 직렬화 형식 = JSON(serde_json) — daemon adapter 가 `serde_json::to_vec` 로 tag1 payload 를 만든다.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
pub enum StructuredEvent {
    /// 어시스턴트 텍스트 증분(스트리밍 델타).
    TextDelta {
        text: String,
        turn_id: Option<String>,
        message_id: Option<String>,
    },
    /// 직렬화된 인자(backend별 스키마 그대로).
    ToolCall {
        name: String,
        args_json: String,
        /// 호출 식별자(권한 UX·결과 매칭용). claude tool_use id 등.
        id: Option<String>,
        turn_id: Option<String>,
        message_id: Option<String>,
    },
    Usage {
        #[ts(type = "number")]
        input_tokens: u64,
        #[ts(type = "number")]
        output_tokens: u64,
        turn_id: Option<String>,
    },
    /// 한 메시지(turn 응답) 종료 신호.
    MessageDone {
        turn_id: Option<String>,
        message_id: Option<String>,
    },
    /// backend 가 보고한 오류(스트림 내부 오류 — 종료 아님).
    Error { message: String },
    /// 위 정형 variant 로 안 잡히는 backend별 이벤트의 탈출구(forward-compat).
    /// kind=종류 태그, json=원본 직렬화 payload(프론트가 kind 로 분기·해석).
    Structured { kind: String, json: String },
}

/// 출력 청크 — 종류 불가지(설계 §2).
/// (구조화 turn 단위 출력은 TUI↔구조화 스위칭 모드 설계 때 실제 채움 — 지금은 형태만 연다.)
///
/// ※S15/ADR-0045: tag1 구조화 이벤트는 이 타입이 아니라 위 [`StructuredEvent`]로 흐른다(필드 유실 0
/// 미러). 이 `OutputChunk`는 GetSnapshot 스냅샷(AgentEvent::Output/Snapshot) 계약 전용으로 남는다.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub enum OutputChunk {
    /// 콘솔 raw 바이트(현 유일 실사용). JSON 경로엔 안 실림 — codec binary frame 전용.
    TerminalBytes(
        #[serde(with = "serde_bytes")]
        #[ts(type = "number[]")]
        Vec<u8>,
    ),
    TextDelta(String),
    Usage {
        input_tokens: u64,
        output_tokens: u64,
    },
    ToolCall {
        name: String,
        args_json: String,
    },
    /// 임의 구조화 페이로드(forward-compat 탈출구).
    Structured {
        kind: String,
        json: String,
    },
}

// ── request_id 추출(request/reply 상관, wire 계약) ────────────────────────────────────
/// 명령에 실린 request_id 를 꺼낸다. side-effect 명령(Spawn/Kill/…)은 모두 request_id 를 갖지만,
/// 일부(Subscribe/Unsubscribe/Resize)는 request_id 가 없다(데몬이 reply 를 안 보냄) → `None`.
/// (핸드셰이크는 이 표에 없다 — 명령이 아니라 네트워크 lib 소유 프레임이라 이 함수에 오지 않는다.
///  ADR-0129 0-4.)
///
/// ★계약★: 데몬 클라이언트의 pending 매칭(`send_command`, src-tauri)은 reply 를 기대하므로
/// request_id 가 있는 명령에만 쓴다. None 인 명령을 넣으면 매칭할 키가 없어 영구 pending(hang)이
/// 되므로 호출자가 None 을 거른다.
pub fn command_request_id(cmd: &AgentCommand) -> Option<RequestId> {
    match cmd {
        AgentCommand::Spawn { request_id, .. }
        | AgentCommand::Kill { request_id, .. }
        | AgentCommand::Interrupt { request_id, .. }
        | AgentCommand::WriteStdin { request_id, .. }
        | AgentCommand::AcquireInput { request_id, .. }
        | AgentCommand::ReleaseInput { request_id, .. }
        | AgentCommand::ListAgents { request_id }
        | AgentCommand::StopDaemon { request_id, .. }
        | AgentCommand::SpawnByCwd { request_id, .. }
        | AgentCommand::ListProfiles { request_id }
        | AgentCommand::CreateProfile { request_id, .. }
        | AgentCommand::DeleteProfile { request_id, .. }
        | AgentCommand::SpawnProfile { request_id, .. }
        | AgentCommand::SetProfileAutoRestore { request_id, .. }
        // 트리 rename(ADR-0061 리치화) — Ack 매칭 대상(SetProfileAutoRestore 와 동형).
        | AgentCommand::RenameProfile { request_id, .. }
        // 트리 reparent(ADR-0072 계층) — Ack 매칭 대상(RenameProfile 와 동형).
        | AgentCommand::ReparentProfile { request_id, .. }
        | AgentCommand::GetSnapshot { request_id, .. }
        // 프리셋 CRUD(ADR-0061) — 넷 다 request_id 동봉(reply 매칭 대상).
        | AgentCommand::ListPresets { request_id }
        | AgentCommand::CreatePreset { request_id, .. }
        | AgentCommand::DeletePreset { request_id, .. }
        // 프리셋 rename(ADR-0061 리치화) — Ack 매칭 대상.
        | AgentCommand::RenamePreset { request_id, .. }
        // 봉투 포맷 전역 스위치(ADR-0096) — Ack 매칭 대상(데몬이 상태 변경 후 Ack echo).
        | AgentCommand::SetEnvelopeFormat { request_id, .. }
        // 명령 버스 등록 wire(ADR-0134/0135) — 셋 다 답장을 기다린다. 등록·차분은 Ack, 조회는
        //   전용 reply CommandList 로 온다(아래 event_reply_request_id 가 그 짝).
        | AgentCommand::RegisterCommands { request_id, .. }
        | AgentCommand::UpdateCommands { request_id, .. }
        | AgentCommand::ListCommands { request_id } => Some(*request_id),
        // request_id 없는 명령 — reply 매칭 대상 아님(데몬이 전용 reply 를 안 echo).
        AgentCommand::Resize { .. }
        | AgentCommand::Subscribe { .. }
        | AgentCommand::Unsubscribe { .. } => None,
    }
}

/// reply 이벤트에 실린 request_id 를 꺼낸다(매칭용). 전용 reply variant(Ack/Spawned/Created/
/// SubscribeAck-는 request_id 없음/AgentList/ProfileList/Snapshot/Error)만 request_id 를 echo 한다 —
/// broadcast(AgentListUpdated/StatusChanged/…)는 `None` 이라 pending 매칭을 우회한다(편승 매칭 제거).
///
/// ★Error 분기★: `Error{request_id: Some(_)}` = 특정 명령 실패(매칭해 reject), `Error{request_id: None}`
/// = 명령 무관 오류(broadcast 성격, 매칭 안 함). SubscribeAck 는 request_id 가 없어(agent_id 기반) 여기
/// None — 데몬 클라이언트의 pending 매칭 대상이 아니다(Subscribe 는 request_id 없는 명령).
pub fn event_reply_request_id(ev: &AgentEvent) -> Option<RequestId> {
    match ev {
        AgentEvent::Ack { request_id }
        | AgentEvent::AgentList { request_id, .. }
        | AgentEvent::ProfileList { request_id, .. }
        // PresetList = 전용 reply(request_id echo, ADR-0061). PresetListUpdated 는 broadcast(아래 None).
        | AgentEvent::PresetList { request_id, .. }
        | AgentEvent::Snapshot { request_id, .. }
        | AgentEvent::Created { request_id, .. }
        // ★CommandList 는 상관 대상이다★(ADR-0134) — ListCommands 조회의 전용 reply라 AgentList/
        //   ProfileList/PresetList 와 같은 자리다. 여기서 None 을 고르면 셸이 확실히 매달린다: 위
        //   command_request_id 가 ListCommands 에 Some 을 돌려주므로 pending 매칭이 슬롯을 만드는데,
        //   그걸 깨울 짝이 없어져 연결이 끊길 때까지 안 풀린다. 두 함수는 명령↔답장 쌍마다 같이
        //   움직여야 하는 한 쌍이다 — 이 쌍의 고정은 이 crate 의 테스트가 박는다
        //   (`cargo test -p engram-dashboard-protocol`, CI 가 항상 실행).
        | AgentEvent::CommandList { request_id, .. }
        | AgentEvent::Spawned { request_id, .. } => Some(*request_id),
        AgentEvent::Error { request_id, .. } => *request_id,
        // request_id 없는 이벤트(broadcast 또는 agent_id 기반) — pending 매칭 대상 아님.
        AgentEvent::Hello { .. }
        | AgentEvent::SubscribeAck { .. }
        | AgentEvent::Output { .. }
        | AgentEvent::ReplayComplete { .. }
        | AgentEvent::StatusChanged { .. }
        | AgentEvent::AgentListUpdated { .. }
        | AgentEvent::RestoreResult { .. }
        | AgentEvent::InputLeaseChanged { .. }
        | AgentEvent::ProfileListUpdated { .. }
        // PresetListUpdated = broadcast(request_id 없음, ADR-0061) — pending 매칭 대상 아님.
        | AgentEvent::PresetListUpdated { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn structured_event_roundtrip_all_variants() {
        let cases = vec![
            StructuredEvent::TextDelta {
                text: "hello".into(),
                turn_id: Some("t1".into()),
                message_id: None,
            },
            StructuredEvent::ToolCall {
                name: "read".into(),
                args_json: r#"{"path":"/x"}"#.into(),
                id: Some("call_1".into()),
                turn_id: None,
                message_id: Some("m1".into()),
            },
            StructuredEvent::Usage {
                input_tokens: 123,
                output_tokens: 456,
                turn_id: Some("t2".into()),
            },
            StructuredEvent::MessageDone {
                turn_id: Some("t3".into()),
                message_id: Some("m2".into()),
            },
            StructuredEvent::Error {
                message: "stream error".into(),
            },
            StructuredEvent::Structured {
                kind: "custom".into(),
                json: r#"{"k":1}"#.into(),
            },
        ];
        for ev in cases {
            let json = serde_json::to_string(&ev).expect("직렬화 성공");
            assert!(
                json.contains("\"type\""),
                "internally-tagged 판별자 누락: {json}"
            );
            let back: StructuredEvent = serde_json::from_str(&json).expect("역직렬화 성공");
            assert_eq!(ev, back, "round-trip 무손실(필드 유실 0)");
        }
    }

    /// optional turn_id/message_id 가 None 일 때도 정확히 None 으로 복원되는지(교체성 — codex/gemini 가
    /// 못 채우는 필드가 임의 값으로 채워지면 안 됨).
    #[test]
    fn structured_event_optional_fields_preserve_none() {
        let ev = StructuredEvent::TextDelta {
            text: "x".into(),
            turn_id: None,
            message_id: None,
        };
        let json = serde_json::to_string(&ev).unwrap();
        let back: StructuredEvent = serde_json::from_str(&json).unwrap();
        match back {
            StructuredEvent::TextDelta {
                turn_id,
                message_id,
                ..
            } => {
                assert!(turn_id.is_none() && message_id.is_none(), "None 보존");
            }
            _ => panic!("variant 불일치"),
        }
    }

    // ── 프리셋 wire 계약(ADR-0061) — JSON envelope golden + round-trip ─────────────
    //
    // 프리셋 CRUD 가 프로필과 동형 형태(variant 이름 태그 + 필드)로 직렬화되는지 고정한다 — wire
    // 포맷이 조용히 바뀌면(필드 개명/누락) 프론트 미러가 깨지므로 golden 문자열로 회귀를 막는다.

    #[test]
    fn create_preset_command_json_golden() {
        let request_id = RequestId(Uuid::nil());
        let cmd = AgentCommand::CreatePreset {
            cwd: "C:/proj".into(),
            request_id,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(
            json,
            r#"{"CreatePreset":{"cwd":"C:/proj","request_id":"00000000-0000-0000-0000-000000000000"}}"#,
            "CreatePreset wire 형태가 golden 과 불일치"
        );
        let back: AgentCommand = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            back,
            AgentCommand::CreatePreset { cwd, .. } if cwd == "C:/proj"
        ));
    }

    #[test]
    fn list_delete_preset_commands_roundtrip() {
        let cases = vec![
            AgentCommand::ListPresets {
                request_id: RequestId(Uuid::nil()),
            },
            AgentCommand::DeletePreset {
                preset_id: Uuid::nil(),
                request_id: RequestId(Uuid::nil()),
            },
        ];
        for cmd in cases {
            let json = serde_json::to_string(&cmd).unwrap();
            let back: AgentCommand = serde_json::from_str(&json).unwrap();
            assert_eq!(json, serde_json::to_string(&back).unwrap());
        }
    }

    #[test]
    fn preset_list_events_json_golden_and_roundtrip() {
        let preset = Preset {
            id: Uuid::nil(),
            cwd: "C:/proj".into(),
            name: None,
        };
        let list = AgentEvent::PresetList {
            request_id: RequestId(Uuid::nil()),
            presets: vec![preset.clone()],
        };
        let list_json = serde_json::to_string(&list).unwrap();
        assert_eq!(
            list_json,
            r#"{"PresetList":{"request_id":"00000000-0000-0000-0000-000000000000","presets":[{"id":"00000000-0000-0000-0000-000000000000","cwd":"C:/proj","name":null}]}}"#,
            "PresetList wire 형태가 golden 과 불일치"
        );

        let updated = AgentEvent::PresetListUpdated {
            presets: vec![preset],
        };
        let updated_json = serde_json::to_string(&updated).unwrap();
        assert_eq!(
            updated_json,
            r#"{"PresetListUpdated":{"presets":[{"id":"00000000-0000-0000-0000-000000000000","cwd":"C:/proj","name":null}]}}"#,
            "PresetListUpdated wire 형태가 golden 과 불일치"
        );

        for json in [list_json, updated_json] {
            let back: AgentEvent = serde_json::from_str(&json).unwrap();
            assert_eq!(json, serde_json::to_string(&back).unwrap());
        }
    }

    #[test]
    fn rename_preset_command_json_golden_and_roundtrip() {
        let cmd = AgentCommand::RenamePreset {
            preset_id: Uuid::nil(),
            name: Some("내 프리셋".into()),
            request_id: RequestId(Uuid::nil()),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(
            json,
            r#"{"RenamePreset":{"preset_id":"00000000-0000-0000-0000-000000000000","name":"내 프리셋","request_id":"00000000-0000-0000-0000-000000000000"}}"#,
            "RenamePreset wire 형태가 golden 과 불일치"
        );
        let back: AgentCommand = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            back,
            AgentCommand::RenamePreset { name: Some(ref n), .. } if n == "내 프리셋"
        ));

        let clear = AgentCommand::RenamePreset {
            preset_id: Uuid::nil(),
            name: None,
            request_id: RequestId(Uuid::nil()),
        };
        let clear_json = serde_json::to_string(&clear).unwrap();
        let clear_back: AgentCommand = serde_json::from_str(&clear_json).unwrap();
        assert_eq!(clear_json, serde_json::to_string(&clear_back).unwrap());
    }

    #[test]
    fn rename_profile_command_json_golden_and_roundtrip() {
        let cmd = AgentCommand::RenameProfile {
            profile_id: Uuid::nil(),
            name: Some("내 에이전트".into()),
            request_id: RequestId(Uuid::nil()),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(
            json,
            r#"{"RenameProfile":{"profile_id":"00000000-0000-0000-0000-000000000000","name":"내 에이전트","request_id":"00000000-0000-0000-0000-000000000000"}}"#,
            "RenameProfile wire 형태가 golden 과 불일치"
        );
        let back: AgentCommand = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            back,
            AgentCommand::RenameProfile { name: Some(ref n), .. } if n == "내 에이전트"
        ));
    }

    #[test]
    fn set_envelope_format_command_json_golden_and_roundtrip() {
        let cmd = AgentCommand::SetEnvelopeFormat {
            format: EnvelopeFormat::Xml,
            request_id: RequestId(Uuid::nil()),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(
            json,
            r#"{"SetEnvelopeFormat":{"format":"xml","request_id":"00000000-0000-0000-0000-000000000000"}}"#,
            "SetEnvelopeFormat wire 형태가 golden 과 불일치(format 은 소문자 xml 이어야 — invoke JSON 계약)"
        );
        let back: AgentCommand = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            back,
            AgentCommand::SetEnvelopeFormat {
                format: EnvelopeFormat::Xml,
                ..
            }
        ));

        let colon = AgentCommand::SetEnvelopeFormat {
            format: EnvelopeFormat::Colon,
            request_id: RequestId(Uuid::nil()),
        };
        let colon_json = serde_json::to_string(&colon).unwrap();
        assert_eq!(
            colon_json,
            r#"{"SetEnvelopeFormat":{"format":"colon","request_id":"00000000-0000-0000-0000-000000000000"}}"#,
            "colon variant 도 소문자여야"
        );
        let colon_back: AgentCommand = serde_json::from_str(&colon_json).unwrap();
        assert_eq!(colon_json, serde_json::to_string(&colon_back).unwrap());
    }

    #[test]
    fn envelope_format_default_is_xml_and_lowercase_deserializes() {
        assert_eq!(
            EnvelopeFormat::default(),
            EnvelopeFormat::Xml,
            "기본 봉투 포맷 = xml(ADR-0103 기본 flip — 데몬 운영 기본과 정합)"
        );
        let xml: EnvelopeFormat = serde_json::from_str(r#""xml""#).unwrap();
        assert_eq!(xml, EnvelopeFormat::Xml);
        let colon: EnvelopeFormat = serde_json::from_str(r#""colon""#).unwrap();
        assert_eq!(colon, EnvelopeFormat::Colon);
    }

    #[test]
    fn reparent_profile_command_json_golden_and_roundtrip() {
        let child = Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        let parent = Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap();

        let cmd = AgentCommand::ReparentProfile {
            child_id: child,
            parent_id: Some(parent),
            request_id: RequestId(Uuid::nil()),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(
            json,
            r#"{"ReparentProfile":{"child_id":"11111111-1111-1111-1111-111111111111","parent_id":"22222222-2222-2222-2222-222222222222","request_id":"00000000-0000-0000-0000-000000000000"}}"#,
            "ReparentProfile(Some) wire 형태가 golden 과 불일치"
        );
        let back: AgentCommand = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            back,
            AgentCommand::ReparentProfile { parent_id: Some(p), .. } if p == parent
        ));

        let clear = AgentCommand::ReparentProfile {
            child_id: child,
            parent_id: None,
            request_id: RequestId(Uuid::nil()),
        };
        let clear_json = serde_json::to_string(&clear).unwrap();
        assert_eq!(
            clear_json,
            r#"{"ReparentProfile":{"child_id":"11111111-1111-1111-1111-111111111111","parent_id":null,"request_id":"00000000-0000-0000-0000-000000000000"}}"#,
            "ReparentProfile(None) wire 형태가 golden 과 불일치"
        );
        let clear_back: AgentCommand = serde_json::from_str(&clear_json).unwrap();
        assert!(matches!(
            clear_back,
            AgentCommand::ReparentProfile {
                parent_id: None,
                ..
            }
        ));
    }

    // ── 명령 버스 등록 wire(ADR-0134/0135) ─────────────────────────────────────────
    //
    // 이 구획은 **아직 배선이 없는** variant 를 지킨다 — 보내는 코드가 없으니 형태가 조용히 틀려도
    // 런타임에 아무 신호가 없고, Step 2 배선이 붙는 날에야 터진다. golden 이 그때까지의 유일한 벽이다.

    /// `#[ts(type = "…")]` 로 손으로 적은 TypeScript 텍스트를 Rust 모양에 묶는다.
    ///
    /// ★이 경로의 유일한 drift 위험이 이 문자열이다★ — `CommandDecl` 은 도구 crate 소유라 `TS` 를
    /// 구현하지 않고(그 crate 의 외부 의존을 최소로 두려는 의도), 그래서 바인딩 텍스트를 ts-rs 가
    /// 파생하지 않고 **사람이 적는다.** 필드가 늘거나 개명되면 Rust 는 컴파일되고 생성 TS 만 거짓말을
    /// 하게 되므로, 여기서 둘을 대조한다.
    #[test]
    fn command_decl_hand_written_ts_matches_rust_shape() {
        use std::collections::BTreeSet;

        // 두 attribute(`RegisterCommands.decls` · `UpdateCommands.added`)에 적은 것과 같은 문자열.
        const DECLS_TS: &str = "Array<{ name: string, help: string }>";

        let inlined = <AgentCommand as TS>::inline();
        assert!(
            inlined.matches(DECLS_TS).count() == 2,
            "생성 TS 가 손으로 적은 텍스트와 불일치(등록 2자리) — attribute 를 고쳤으면 여기도 고친다:\n{inlined}"
        );

        let decl = CommandDecl {
            name: "agent.spawn".into(),
            help: r#"{"name":"agent.spawn"}"#.into(),
        };
        let json = serde_json::to_value(&decl).expect("CommandDecl 직렬화");
        let rust_fields: BTreeSet<String> = json
            .as_object()
            .expect("CommandDecl 은 JSON object 로 나간다")
            .iter()
            .map(|(k, v)| {
                assert!(
                    v.is_string(),
                    "{k} 가 string 이 아니면 위 TS 텍스트가 거짓말이다"
                );
                k.clone()
            })
            .collect();

        // ★위 rust_fields 만으론 안 잡히는 구멍★: `#[serde(skip_serializing_if = "Option::is_none")]`
        //   필드는 값이 None 이면 직렬화 결과 자체에서 키가 사라진다 — 새 옵셔널 필드가 이 fixture 에서
        //   None 이면 rust_fields 에 안 뜨고, ts_fields 도 그 필드를 안 적었으면(TS 갱신을 잊음) 둘 다
        //   "없음"으로 일치해 아래 첫 assert_eq 가 조용히 통과한다. Debug 는 serde 속성과 무관하게 모든
        //   필드를 항상 찍으므로(derive 는 필드를 가리지 않는다) 이걸로 실제 필드 집합을 다시 뽑아
        //   ts_fields 와 대조한다. pretty-print(`{:#?}`) 최상위 필드 줄만 거른다(4칸 들여쓰기, 더 깊은
        //   들여쓰기는 중첩 값이라 제외) — 필드 값 안의 `,`·`:` 에 흔들리지 않는다.
        let debug_fields: BTreeSet<String> = format!("{decl:#?}")
            .lines()
            .filter_map(|line| {
                let rest = line.strip_prefix("    ")?;
                if rest.starts_with(' ') {
                    return None;
                }
                let (name, _) = rest.split_once(':')?;
                Some(name.trim().to_string())
            })
            .collect();

        // 손으로 적은 텍스트에서 `<이름>: <타입>` 의 이름만 뽑는다.
        let ts_fields: BTreeSet<String> = DECLS_TS
            .trim_start_matches("Array<{")
            .trim_end_matches("}>")
            .split(',')
            .filter(|s| !s.trim().is_empty())
            .map(|pair| {
                let (name, ty) = pair.split_once(':').expect("`이름: 타입` 형태");
                assert_eq!(ty.trim(), "string", "{name} 의 TS 타입이 string 이 아니다");
                name.trim().to_string()
            })
            .collect();

        assert_eq!(
            rust_fields, ts_fields,
            "CommandDecl 의 Rust 필드와 손으로 적은 TS 필드가 갈렸다"
        );
        assert_eq!(
            debug_fields, ts_fields,
            "CommandDecl 의 실제 필드(Debug 기준, skip_serializing_if 로도 안 가려짐)와 손으로 적은 \
             TS 필드가 갈렸다 — 위 rust_fields 비교는 값이 None 이라 직렬화에서 빠진 필드를 못 잡는다"
        );
    }

    /// 등록 패킷이 선언을 실어 왕복한다 — `owner` 는 맨 문자열, `help` 는 **바이트 그대로** 보존된다.
    #[test]
    fn register_commands_round_trips_declarations() {
        let help = r#"{"name":"agent.spawn","effect":"Write","since":1}"#;
        let cmd = AgentCommand::RegisterCommands {
            owner: OwnerToken::new("shell"),
            decls: vec![CommandDecl {
                name: "agent.spawn".into(),
                help: help.into(),
            }],
            catalog_version: 1,
            request_id: RequestId(Uuid::nil()),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(
            json,
            r#"{"RegisterCommands":{"owner":"shell","decls":[{"name":"agent.spawn","help":"{\"name\":\"agent.spawn\",\"effect\":\"Write\",\"since\":1}"}],"catalog_version":1,"request_id":"00000000-0000-0000-0000-000000000000"}}"#,
            "RegisterCommands wire 형태가 golden 과 불일치(owner 는 맨 문자열, help 는 통짜 문자열)"
        );

        let back: AgentCommand = serde_json::from_str(&json).unwrap();
        match back {
            AgentCommand::RegisterCommands { owner, decls, .. } => {
                assert_eq!(owner.as_str(), "shell");
                assert_eq!(decls.len(), 1);
                assert_eq!(
                    decls[0].help, help,
                    "help 는 한 글자도 안 바뀌고 건너야 한다 — 중계자가 열어보지 않는다는 계약의 실물"
                );
            }
            other => panic!("variant 불일치: {other:?}"),
        }
    }

    /// 차분은 `added`(모양 포함) + `removed`(이름만)로 갈린다 — 둘을 한 모양으로 합치면 내릴 때
    /// 없는 `help` 를 지어내야 한다.
    #[test]
    fn update_commands_carries_added_shapes_and_removed_names() {
        let cmd = AgentCommand::UpdateCommands {
            owner: OwnerToken::new("shell"),
            added: vec![CommandDecl {
                name: "chat.get".into(),
                help: "{}".into(),
            }],
            removed: vec!["chat.reset".into()],
            request_id: RequestId(Uuid::nil()),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(
            json,
            r#"{"UpdateCommands":{"owner":"shell","added":[{"name":"chat.get","help":"{}"}],"removed":["chat.reset"],"request_id":"00000000-0000-0000-0000-000000000000"}}"#,
            "UpdateCommands wire 형태가 golden 과 불일치"
        );
        let back: AgentCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(json, serde_json::to_string(&back).unwrap());
    }

    /// 조회 왕복 — 자취(`available=false`)도 목록에 실려야 §4-② 의 두 오류가 갈린다.
    #[test]
    fn list_commands_and_command_list_round_trip() {
        let req = AgentCommand::ListCommands {
            request_id: RequestId(Uuid::nil()),
        };
        assert_eq!(
            serde_json::to_string(&req).unwrap(),
            r#"{"ListCommands":{"request_id":"00000000-0000-0000-0000-000000000000"}}"#
        );

        let reply = AgentEvent::CommandList {
            request_id: RequestId(Uuid::nil()),
            entries: vec![
                CommandListEntry {
                    name: "agent.spawn".into(),
                    help: "{}".into(),
                    available: true,
                },
                CommandListEntry {
                    name: "tab.create".into(),
                    help: "{}".into(),
                    available: false,
                },
            ],
        };
        let json = serde_json::to_string(&reply).unwrap();
        assert_eq!(
            json,
            r#"{"CommandList":{"request_id":"00000000-0000-0000-0000-000000000000","entries":[{"name":"agent.spawn","help":"{}","available":true},{"name":"tab.create","help":"{}","available":false}]}}"#,
            "CommandList wire 형태가 golden 과 불일치"
        );
        let back: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(json, serde_json::to_string(&back).unwrap());
    }

    /// ★명령↔답장 쌍 박제(ADR-0134)★: `AgentCommand::ListCommands` 가 pending 슬롯을 만드는 쪽이고
    /// `AgentEvent::CommandList` 가 그걸 깨우는 쪽이다. 한쪽만 고치면(예: 새 reply variant 를
    /// broadcast 로 잘못 분류) 그 왕복은 연결이 끊길 때까지 안 풀린다. 동형 검증이 `src-tauri`
    /// `daemon_client::protocol_state` 에도 있었으나 그 lib 테스트 타깃은 로컬/CI 모두
    /// 0xc0000139(ENTRYPOINT_NOT_FOUND)로 실행되지 않는다 — 실제로 도는 건 이 crate 테스트뿐이다.
    #[test]
    fn list_commands_reply_request_id_pairing() {
        let r = RequestId::new();
        let cmd = AgentCommand::ListCommands { request_id: r };
        assert_eq!(
            command_request_id(&cmd),
            Some(r),
            "ListCommands 는 request_id 동봉 — pending 슬롯을 만드는 쪽"
        );
        let reply = AgentEvent::CommandList {
            request_id: r,
            entries: vec![],
        };
        assert_eq!(
            event_reply_request_id(&reply),
            Some(r),
            "CommandList 는 ListCommands 의 전용 reply — 같은 request_id 로 매칭돼야 슬롯이 깨어난다"
        );
    }

    /// `request_id` 가 빠진 패킷은 **거절돼야 한다** — 이 crate 의 `RequestId::default()` 는 새 v4 를
    /// 찍으므로 `#[serde(default)]` 가 붙으면 조용히 짝 없는 상관 키가 생긴다(답장이 영영 안 붙는다).
    /// 그 attribute 가 실수로 들어오면 이 단언이 먼저 깨진다.
    #[test]
    fn registration_variants_reject_missing_request_id() {
        for json in [
            r#"{"RegisterCommands":{"owner":"shell","decls":[],"catalog_version":1}}"#,
            r#"{"UpdateCommands":{"owner":"shell","added":[],"removed":[]}}"#,
            r#"{"ListCommands":{}}"#,
        ] {
            assert!(
                serde_json::from_str::<AgentCommand>(json).is_err(),
                "request_id 부재는 기본값으로 흡수되면 안 된다: {json}"
            );
        }
    }

    #[test]
    fn agent_profile_wire_roundtrip_includes_parent_id() {
        use crate::domain::{AgentProfile as WireProfile, AgentSpawnCommand, RestartPolicy};

        let parent = Uuid::parse_str("33333333-3333-3333-3333-333333333333").unwrap();
        let mk = |parent_id: Option<Uuid>| WireProfile {
            id: Uuid::nil(),
            name: "p".into(),
            display_name: None,
            parent_id,
            command: AgentSpawnCommand::Shell {
                program: "cmd.exe".into(),
                args: vec![],
            },
            cwd: "C:/proj".into(),
            env: vec![],
            claude_session_id: None,
            old_session_ids: vec![],
            epoch: 0,
            auto_restore: true,
            restart_policy: RestartPolicy::Always,
            restart_count: 0,
            failed_reason: None,
            created_at: 1,
            last_active: 1,
            last_start_at: None,
        };

        for parent_id in [Some(parent), None] {
            let p = mk(parent_id);
            let json = serde_json::to_string(&p).unwrap();
            let back: WireProfile = serde_json::from_str(&json).unwrap();
            assert_eq!(back.parent_id, parent_id, "parent_id 왕복 보존");
            assert_eq!(json, serde_json::to_string(&back).unwrap());
        }

        let legacy = r#"{
            "id": "00000000-0000-0000-0000-000000000000",
            "name": "legacy",
            "command": { "kind": "Shell", "program": "cmd.exe", "args": [] },
            "cwd": "C:/proj",
            "env": [],
            "claude_session_id": null,
            "old_session_ids": [],
            "epoch": 0,
            "auto_restore": true,
            "restart_policy": "Always",
            "restart_count": 0,
            "failed_reason": null,
            "created_at": 1,
            "last_active": 1,
            "last_start_at": null
        }"#;
        let p: WireProfile = serde_json::from_str(legacy).expect("parent_id 없는 wire 역직렬화");
        assert_eq!(p.parent_id, None, "parent_id 부재 → None(루트)");
    }
}
