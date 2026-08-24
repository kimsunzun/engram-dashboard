//! wire 메시지 — UI→core [`AgentCommand`], core→UI [`AgentEvent`].
//! 둘 다 externally-tagged JSON(serde 기본).

use engram_dashboard_command::{CommandDecl, CommandEnvelope, CommandReply, OwnerToken};
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

    // ── 명령 버스 등록 wire(ADR-0155/0156 · TRD §3-7) ───────────────────────────────
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
    /// `removed` 가 이름만 나르는 것은 의도다 — 내릴 때 모양은 필요 없다. 내린 이름은 명부에서 **자리째
    /// 지워지고**(ADR-0150 결정 3) 그 뒤로 `UNKNOWN_COMMAND` 로 답한다 — 자취로 남기면 붙어 있는 주인이
    /// 이름을 바꿔 가며 자기 몫 상한을 영구히 채울 수 있다.
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

    /// 이 클라이언트가 내는 **명령 요청** — 답은 [`AgentEvent::CommandReply`] 로 온다(ADR-0155 결정 3).
    ///
    /// ★[`AgentEvent::CommandRequest`] 의 거울상이고 봉투 타입이 **같다**★ — 같은 어휘가 두 방향으로
    /// 흐르고 **어느 연결에 썼는가가 방향**이다(TRD §3-2). 그래서 방향 필드도, 방향마다 다른 봉투도 없다.
    /// ★상관 키가 봉투 안에 있어 이 variant 엔 `request_id` 칸이 없다★ — 형제들과 달라 보이지만
    /// [`command_request_id`] 가 봉투에서 꺼내 `Some` 을 주므로 pending 매칭 대상은 맞다. 그 짝이
    /// [`event_reply_request_id`] 의 `CommandReply` 갈래다 — **둘 중 하나만 `Some` 이면 왕복이 안 닫힌다.**
    ///
    /// `envelope.owner` 는 **목적지** 토큰이지 보낸 이가 아니다 — 최종 지목은 데몬이 자기 명부로 한다
    /// (ADR-0154). `envelope.args` 는 데몬이 파싱하지 않고 통과시킨다(ADR-0081 「데몬 opaque 유지」).
    Command {
        #[ts(
            type = "{ name: string, request_id: string, owner: string, proto_ver: number, args: unknown }"
        )]
        envelope: CommandEnvelope,
    },

    /// 데몬이 배달한 명령([`AgentEvent::CommandRequest`])의 **결말**.
    ///
    /// ★이것은 요청이 아니라 답장이다★ — 그래서 자기 `request_id` 칸이 없고 상관 키는 `reply` 안에 있다
    /// (봉투가 받은 그 키 그대로 — 홉마다 새로 만들지 않는다, ADR-0081 「request_id 왕복 보존」).
    /// [`command_request_id`] 가 이 variant 에 `None` 을 주는 것이 그 사실의 표현이다: pending 매칭에 넣으면
    /// 깨울 짝이 없어 영구 pending 이 된다.
    /// ★방향 필드가 없는 것과 짝이다★ — 같은 봉투 어휘가 두 방향으로 흐르고, 어느 연결에 썼는가가 방향이다
    /// (TRD §3-2 · [`CommandEnvelope::owner`] 주석).
    ///
    /// ★TS 칸은 **Rust 가 받아들이는 것**을 적는다 — 내보내는 것만 적으면 거짓이 된다★: 오류 세 칸은 전부
    /// 생략·`null` 이 허용되고(`CommandError` 의 `RawError` 가 `Option<String>` 셋) 계약 밖 필드도 그대로
    /// 통과한다(`#[serde(flatten)] extra` — `deny_unknown_fields` 는 additive 진화를 깨므로 안 단다). 필수
    /// 문자열로 적으면 **유효한 입력을 TS 로는 표현할 수 없다.** 되보내는 방향에도 그 관용이 실재한다 —
    /// 릴레이 홉이 받은 원문을 재직렬화하면 세 칸 중 일부가 빠진 채 나간다(그 `Serialize` 구현).
    CommandOutcome {
        #[ts(
            type = "{ request_id: string, outcome: { Ok: unknown } | { Err: { code?: string | null, message?: string | null, retry?: string | null, [key: string]: unknown } } }"
        )]
        reply: CommandReply,
    },
}

/// 데몬 명부의 **클라이언트 투영** 한 줄([`AgentEvent::CommandList`] 의 원소).
///
/// ★버스 계약이 아니라 wire 계약이라 여기 산다★ — 도구 crate 의 `CommandDecl`(등록 단위)과 필드가
/// 겹치지만 방향과 주인이 다르다: `CommandDecl` 은 주인→데몬이 얹는 것이고, 이쪽은 데몬이 명부를
/// 훑어 내려 주는 것이라 명부만 아는 칸(`available`)이 하나 더 붙는다.
///
/// ★`available` 은 **지금 항상 `true`** 이고 분기 근거로 쓰지 말 것★ — 주인이 끊기면 그 이름이 명부에서
/// 사라져 목록에 아예 실리지 않으므로(ADR-0150 결정 3) 실려 온 항목은 전부 살아 있는 등록이다. `false` 는
/// 도달 불가고, 이 칸으로 가용성을 판정하는 코드는 **판정할 것이 없는 판정**이다. 칸을 떼지 않은 이유는
/// 떼는 것 자체가 wire 계약 변경인데 얻는 것이 없어서다(TRD §3-7).
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
    /// [`AgentCommand::Subscribe`] 가 **거절**됐다 — [`AgentEvent::SubscribeAck`] 의 실패 짝.
    ///
    /// ★왜 `Error` 로 안 보내나(load-bearing — 이 variant 의 존재 이유)★: `Subscribe` 는 `request_id` 가
    /// 없는 명령이라(위 [`command_request_id`]) 그 거절은 `Error{request_id: None}` 으로 나갔고, 그 봉투엔
    /// **주인을 식별할 필드가 없다**. 받는 쪽(src-tauri)은 어느 에이전트의 구독이 깨졌는지 알 수 없어
    /// 자기 single-flight 슬롯을 풀지 못했고, 그 슬롯이 좀비로 남아 **그 에이전트의 Subscribe 가 두 번
    /// 다시 나가지 못했다** — 데몬 재기동 뒤 그 에이전트의 출력이 모든 창에서 영구 두절(실측 2026-08-19).
    /// 그래서 거절에 `agent_id` 를 실어 상관 가능하게 만든다.
    ///
    /// ★계약★: 이 이벤트가 나가면 **그 Subscribe 에 대한 `SubscribeAck` 도 `ReplayComplete` 도 오지
    /// 않는다**(둘 다 거절 지점 이후에만 발행된다). 받는 쪽은 이 사실에 기대어 슬롯을 즉시 해제해도
    /// 안전하다 — 나중에 도착해 오귀속될 응답이 존재하지 않는다.
    ///
    /// ★`reason` 은 사람이 읽는 진단 문자열이다★ — 기계 분기 금지(문구는 예고 없이 바뀐다). 분기해야 하면
    /// 코드 필드를 새로 판다.
    SubscribeFailed {
        #[ts(type = "string")]
        agent_id: AgentId,
        reason: String,
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

    /// [`AgentCommand::ListCommands`] 응답(전용 reply, ADR-0155/0156) — request_id 에코.
    /// broadcast 가 아니다(요청한 연결에만 간다). 데몬 dispatch 가 명부를 훑어 이걸 낸다.
    ///
    /// ★구형 셸 안전은 오직 "전용 reply" 라는 사실에만 기댄다★ — 구형 셸은 `ListCommands` 를 보내지
    /// 않으니 이 답장도 못 받고, 설령 받아도 모르는 externally-tagged variant 키는 조용히 버린다(에러도
    /// 로그도 없음). **이 variant 를 나중에 broadcast(요청 없이 push)로 바꾸는 순간** 모든 구형 셸이
    /// 그 이벤트를 아무 신호 없이 잃는다 — broadcast 로 바꾸려면 이 안전을 다시 설계해야 한다.
    ///
    /// 주인이 끊긴 이름은 실려 오지 않는다 — 명부에서 사라졌기 때문이다(ADR-0150 결정 3). 그래서 목록에
    /// 없는 이름은 「없는 이름」과 「주인이 자리 비움」이 합쳐진 답이다(감수한 손실).
    CommandList {
        request_id: RequestId,
        entries: Vec<CommandListEntry>,
    },

    /// 데몬이 이 클라이언트 앞으로 배달하는 **명령**(ADR-0155 결정 3 의 2단계).
    ///
    /// 이 variant 가 클라이언트를 「데몬 명령 **수신** peer」로 만든다 — ADR-0081 이 「신규 능력」으로 적은
    /// 그것이고, 그 ADR 의 3-variant opaque relay 봉투는 ADR-0155 이 이 통합 봉투로 대체했다.
    ///
    /// ★이 enum 의 유일한 「요청」 variant 다★ — 나머지 18개는 알림이거나 내가 보낸 명령의 답장이다. 그래서
    /// 받는 쪽은 이것만 [`AgentEvent`] 소비 흐름에서 갈라내 인바운드 수신기로 넘기고, 답은
    /// [`AgentCommand::CommandOutcome`] 으로 되돌린다. [`event_reply_request_id`] 가 여기 `None` 을 주는 것이
    /// 계약이다 — `Some` 이면 받는 쪽 pending 매칭이 이 요청을 「내가 기다린 답장」으로 읽고 삼킨다(그 봉투는
    /// 실행되지 않고 사라지고, 보낸 쪽은 마감시각까지 매달린다).
    ///
    /// `envelope.args` 는 데몬이 **파싱하지 않고 통과시킨** 값이다(ADR-0081 「데몬 opaque 유지」 —
    /// ADR-0155 이 그 조항을 살려 두었다). `envelope.owner` 는 **목적지** 토큰이고 보낸 이가 아니다.
    CommandRequest {
        #[ts(
            type = "{ name: string, request_id: string, owner: string, proto_ver: number, args: unknown }"
        )]
        envelope: CommandEnvelope,
    },

    /// [`AgentCommand::Command`] 의 **답장**(전용 reply, ADR-0155 결정 3) — 상관 키는 `reply.request_id` 다.
    ///
    /// ★[`AgentCommand::CommandOutcome`] 과 타입이 같고 방향만 다르다★ — 이쪽은 내가 낸 요청의 답이라
    /// [`event_reply_request_id`] 가 `Some` 을 주고, 저쪽은 내가 보내는 답이라 `None` 이다. 동형이라
    /// 헷갈리기 쉬운 자리이고, 갈림의 근거는 **어느 왕복의 주인이 나인가** 하나다.
    /// ★broadcast 로 바꾸지 말 것★ — [`AgentEvent::CommandList`] 와 같은 이유로 「전용 reply」라는
    /// 사실 위에 구형 셸 안전이 서 있다.
    ///
    /// TS 칸이 광고하는 관용(오류 세 칸 생략·`null` 허용, 계약 밖 필드 통과)의 근거는
    /// [`AgentCommand::CommandOutcome`] 에 적었다 — 같은 타입이라 같은 관용이다.
    ///
    /// ★생산자는 이제 있다★ — 데몬이 자기 명부에서 주인을 찾아 배달하고 그 결말을 이 답장으로 되돌린다
    /// (`engram_dashboard_daemon::command_delivery` · ADR-0154). ★그래도 **프론트까지는 안 닿는다**★ —
    /// 아래 두 조각이 아직 비어 있고, 그 둘은 **같은 변경에서** 서야 한다(마지막 문단).
    ///
    /// 셸은 상관한다: [`command_request_id`] 가 [`AgentCommand::Command`] 에 `Some` 을 주므로
    /// `forward_daemon_command`(`src-tauri/src/commands/agent.rs`)가 **답장 대기** 갈래를 탄다(그 함수엔
    /// variant allowlist 가 없다 — `Subscribe`/`Unsubscribe` 만 막고 나머지는 이 `Some`/`None` 하나로
    /// 갈린다). 답장이 오면 `src-tauri/src/daemon_client/connection.rs` 가 [`event_reply_request_id`] 로
    /// pending 을 깨고 그 이벤트를 **그대로** 돌려준다.
    /// carrier 도 통과시킨다: `TauriTransport.send`(`src/api/tauriTransport.ts`)가 그 invoke 반환을
    /// control 메시지로 올려 `ProtocolClient.handleEvent` 에 넣는다. ★이름 붙은 broadcast 몇 종만
    /// 되만드는 필터는 **push 방향에만** 있다 — 답장 방향은 무필터 통과다★.
    ///
    /// ★그런데 `handleEvent`(`src/api/protocolClient.ts`)에는 이 variant 갈래가 **없다**★ — 마지막 `if`
    /// 를 지나 아무 일 없이 끝난다(else 도 warn 도 throw 도 없는 조용한 폐기).
    /// ★그러면 그 호출의 promise 는 **영영 안 풀린다**★: 프론트 `sendCommand` 는 타이머를 걸지 않고,
    /// `invoke` 가 **성공**으로 끝났으니 `send().catch` 도 안 불린다. 남는 탈출구는 연결 상태 전이나
    /// `close()` 의 일괄 reject 뿐이다. 셸의 30초 답장 상한은 **답장이 안 오는 경우**를 막는 장치라
    /// 여기(답장이 와서 버려진 경우)엔 닿지 않는다.
    ///
    /// ★그러므로 **화면 쪽 producer**(웹뷰가 [`AgentCommand::Command`] 를 내는 자리 — 오늘 `src/` 에 0건)를
    /// 얹는 변경이 `handleEvent` 갈래도 **같은 변경에서** 얹어야 한다★ — 이 파일의 두 상관 함수가 한 쌍으로만
    /// 성립하는 것과 같은 요구가 한 층 아래에서 그대로 반복된다. 그 producer 가 없는 동안 이 구멍은 **도달
    /// 불가**다: 데몬이 배달하는 봉투는 셸이 자기 표에서 실행하고(`daemon_client::inbound`) 답장도 셸이
    /// 받으므로 웹뷰의 pending 을 거치지 않는다.
    CommandReply {
        #[ts(
            type = "{ request_id: string, outcome: { Ok: unknown } | { Err: { code?: string | null, message?: string | null, retry?: string | null, [key: string]: unknown } } }"
        )]
        reply: CommandReply,
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
        // 명령 버스 등록 wire(ADR-0155/0156) — 셋 다 답장을 기다린다. 등록·차분은 Ack, 조회는
        //   전용 reply CommandList 로 온다(아래 event_reply_request_id 가 그 짝).
        | AgentCommand::RegisterCommands { request_id, .. }
        | AgentCommand::UpdateCommands { request_id, .. }
        | AgentCommand::ListCommands { request_id } => Some(*request_id),
        // ★명령 요청은 상관 대상이다 — 키만 봉투 안에 있다★(ADR-0155). 형제들처럼 제 칸이 없다고 여기서
        //   None 을 고르면 셸이 답장을 받고도 깨울 슬롯을 못 만들어 마감시각까지 매달린다. 아래
        //   event_reply_request_id 의 `CommandReply` 갈래와 **한 쌍으로만** 성립한다.
        // ★uuid 는 그대로 옮긴다★ — 홉에서 새 키가 나면 답장이 이 요청에 못 붙는다(`RequestId` 의 From).
        AgentCommand::Command { envelope } => Some(envelope.request_id.into()),
        // request_id 없는 명령 — reply 매칭 대상 아님(데몬이 전용 reply 를 안 echo).
        AgentCommand::Resize { .. }
        | AgentCommand::Subscribe { .. }
        | AgentCommand::Unsubscribe { .. }
        // ★CommandOutcome 은 **내가 보내는 답장**이라 여기 None 이다★ — 상관 키가 `reply` 안에 있지만 그것은
        //   데몬이 나에게 준 요청의 키이고, 내 pending 표의 키가 아니다. Some 을 돌려주면 답장을 보내는 그
        //   순간 그 키로 빈 pending 슬롯이 생겨 연결이 끊길 때까지 남는다.
        | AgentCommand::CommandOutcome { .. } => None,
    }
}

/// reply 이벤트에 실린 request_id 를 꺼낸다(매칭용). **전용 reply**(요청 하나에 붙는 답)만 request_id 를
/// echo 하고, broadcast(AgentListUpdated/StatusChanged/…)는 `None` 이라 pending 매칭을 우회한다(편승 매칭
/// 제거). ★어느 variant 가 어느 쪽인지의 목록은 아래 match 하나다★ — 여기 베껴 두면 variant 가 늘 때
/// 한쪽만 고쳐진다.
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
        // ★CommandList 는 상관 대상이다★(ADR-0155) — ListCommands 조회의 전용 reply라 AgentList/
        //   ProfileList/PresetList 와 같은 자리다. 여기서 None 을 고르면 셸이 확실히 매달린다: 위
        //   command_request_id 가 ListCommands 에 Some 을 돌려주므로 pending 매칭이 슬롯을 만드는데,
        //   그걸 깨울 짝이 없어져 연결이 끊길 때까지 안 풀린다. 두 함수는 명령↔답장 쌍마다 같이
        //   움직여야 하는 한 쌍이다 — 이 쌍의 고정은 이 crate 의 테스트가 박는다
        //   (`cargo test -p engram-dashboard-protocol`, CI 가 항상 실행).
        | AgentEvent::CommandList { request_id, .. }
        | AgentEvent::Spawned { request_id, .. } => Some(*request_id),
        // ★명령 답장도 상관 대상이다 — 위 `AgentCommand::Command` 갈래의 짝★(ADR-0155). 요청이 Some 을
        //   주는데 여기서 None 을 고르면 그 슬롯을 깨울 짝이 없어져 연결이 끊길 때까지 안 풀린다.
        AgentEvent::CommandReply { reply } => Some(reply.request_id.into()),
        AgentEvent::Error { request_id, .. } => *request_id,
        // request_id 없는 이벤트(broadcast 또는 agent_id 기반) — pending 매칭 대상 아님.
        AgentEvent::Hello { .. }
        | AgentEvent::SubscribeAck { .. }
        // SubscribeFailed 도 agent_id 기반이다(Subscribe 는 request_id 없는 명령) — 매칭 대상 아님.
        | AgentEvent::SubscribeFailed { .. }
        | AgentEvent::Output { .. }
        | AgentEvent::ReplayComplete { .. }
        | AgentEvent::StatusChanged { .. }
        | AgentEvent::AgentListUpdated { .. }
        | AgentEvent::RestoreResult { .. }
        | AgentEvent::InputLeaseChanged { .. }
        | AgentEvent::ProfileListUpdated { .. }
        // PresetListUpdated = broadcast(request_id 없음, ADR-0061) — pending 매칭 대상 아님.
        | AgentEvent::PresetListUpdated { .. }
        // ★CommandRequest 는 **들어오는 요청**이라 여기 None 이다(load-bearing)★ — 봉투에 request_id 가
        //   있으니 Some 이 자연스러워 보이지만, 그 키는 **데몬이 만든 요청의 키**이고 내 pending 표의 키가
        //   아니다. Some 을 돌려주면 받는 쪽이 이것을 「내가 기다린 답장」으로 읽어 봉투를 삼킨다 — 명령은
        //   실행되지 않고 보낸 쪽은 마감시각까지 매달린다.
        | AgentEvent::CommandRequest { .. } => None,
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

    // ── 명령 버스 등록 wire(ADR-0155/0156) ─────────────────────────────────────────
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

    /// 조회 왕복. `available=false` 를 함께 굽는 것은 지금 의도다 — 데몬은 그 값을 내지 않지만
    /// (ADR-0150 결정 3) 계약은 두 값을 다 나르고, 이 골든이 그 칸이 조용히 사라지지 않게 붙든다.
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

    /// ★명령↔답장 쌍 박제(ADR-0155)★: `AgentCommand::ListCommands` 가 pending 슬롯을 만드는 쪽이고
    /// `AgentEvent::CommandList` 가 그걸 깨우는 쪽이다. 한쪽만 고치면(예: 새 reply variant 를
    /// broadcast 로 잘못 분류) 그 왕복은 연결이 끊길 때까지 안 풀린다. 동형 검증이 `src-tauri`
    /// `daemon_client::protocol_state` 에도 있는데, **CI 에서 도는 건 이 crate 테스트뿐이다** —
    /// 그쪽 단위 스위트는 아직 CI 에 등재돼 있지 않다(알려진 실패 30건 대기 · 로컬에서는 2026-08-24부터
    /// `cargo test -p engram-dashboard --test lib_unit` 으로 돈다. 옛 사유였던
    /// 0xc0000139(ENTRYPOINT_NOT_FOUND) 즉사는 해소됐다).
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
            last_failure: None,
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
        assert_eq!(
            p.last_failure, None,
            "last_failure 부재 → None(ADR-0172 additive — 옛 wire 가 그대로 통과해야 버전 bump 가 없다)"
        );
    }

    /// ★어휘가 늘 때 옛 피어를 지키는 절반★: 모르는 종류 문자열 하나가 `AgentProfile` **메시지 전체**의
    /// 디코드를 깨면, 그 프로필이 화면에서 통째로 사라진다(필드만 비는 게 아니다). 프론트 표의
    /// `table[kind] ?? Other` 와 짝을 이룬다.
    // ADR-0172
    #[test]
    fn an_unknown_failure_kind_is_absorbed_instead_of_failing_the_whole_profile() {
        use crate::domain::{AgentFailureKind, AgentProfile as WireProfile};

        let with_future_kind = r#"{
            "id": "00000000-0000-0000-0000-000000000000",
            "name": "future",
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
            "last_failure": "SomeKindFromANewerDaemon",
            "created_at": 1,
            "last_active": 1,
            "last_start_at": null
        }"#;
        let p: WireProfile = serde_json::from_str(with_future_kind)
            .expect("모르는 종류가 메시지를 깨뜨리면 안 된다");
        assert_eq!(p.name, "future", "프로필의 나머지 칸이 온전히 남는다");
        assert_eq!(
            p.last_failure,
            Some(AgentFailureKind::Other),
            "모르는 종류는 「그 밖」으로 흡수된다(재시도 가능 = fail-open)"
        );

        // 아는 어휘는 그대로 통과한다(흡수가 전부를 뭉개지 않는다).
        let known = with_future_kind.replace("SomeKindFromANewerDaemon", "NoConversationToResume");
        let p: WireProfile = serde_json::from_str(&known).expect("아는 종류");
        assert_eq!(
            p.last_failure,
            Some(AgentFailureKind::NoConversationToResume)
        );

        // ★흡수는 **모르는 문자열** 에만 열려 있다★: 형식 위반까지 삼키면 망가진 값이 `Other` 로 둔갑해
        //   실패가 없는 항목에 실패 문구가 뜬다(흡수가 아니라 날조).
        for bad in ["42", "[]", "{}", "true"] {
            let broken = with_future_kind.replace("\"SomeKindFromANewerDaemon\"", bad);
            assert!(
                serde_json::from_str::<WireProfile>(&broken).is_err(),
                "비-문자열 {bad} 은 흡수 대상이 아니다"
            );
        }
    }

    /// ★wire 어휘와 **생성된 TS 유니온**이 같아야 한다★: 프론트가 받는 타입은 ts-rs 가 만들고, 실제
    /// JSON 은 `as_wire_str` 가 만든다. 둘이 갈리면 프론트 표가 모든 값을 「그 밖」으로 흡수해 문구가
    /// 조용히 하나로 뭉개진다.
    ///
    /// ★테스트 안에 어휘를 다시 적지 않는다(그러면 아무것도 안 재는 테스트가 된다)★: 예전 형태는 튜플에
    ///   손으로 쓴 문자열을 비교했는데, 변형을 `Other`→`Unknown` 으로 바꾸고 arm 이 `"Other"` 를 계속
    ///   내보내도 **통과**했다 — 정확히 이 doc 이 막는다고 적힌 사고다. 이제 한쪽은 생성물 파일에서,
    ///   다른 쪽은 매크로가 `stringify!` 로 만든 목록에서 온다.
    // ADR-0172
    #[test]
    fn failure_kind_vocabulary_matches_the_generated_ts_union() {
        use crate::domain::AgentFailureKind;

        // 생성물에서 유니온 멤버를 뽑는다(ts-rs 가 변형 이름으로 만든 쪽).
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/bindings/AgentFailureKind.ts");
        let src = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("생성된 바인딩을 읽어야 한다({path}): {e}"));
        let decl = src
            .lines()
            .find(|l| l.starts_with("export type AgentFailureKind"))
            .expect("유니온 선언 줄");
        // `export type X = "A" | "B";` → 홀수 조각이 멤버다.
        let ts_members: Vec<&str> = decl.split('"').skip(1).step_by(2).collect();

        let wire_names: Vec<&str> = AgentFailureKind::wire_names().collect();
        assert_eq!(
            ts_members, wire_names,
            "TS 유니온과 wire 어휘가 갈렸다 — 프론트가 받는 값이 표에 없어 전부 「그 밖」으로 접힌다"
        );

        // 그리고 그 어휘가 실제로 왕복한다(직렬화 문자열 = 그 이름, 역직렬화 = 그 변형).
        for (kind, name) in AgentFailureKind::all() {
            assert_eq!(serde_json::to_string(kind).unwrap(), format!("\"{name}\""));
            assert_eq!(
                &serde_json::from_str::<AgentFailureKind>(&format!("\"{name}\"")).unwrap(),
                kind,
                "왕복이 성립해야 한다"
            );
        }
    }

    /// ★이스케이프가 든 값도 디코드돼야 한다★: 빌린 `&str` 로 받던 시절엔 `"Other"` 하나로
    /// `AgentProfile` **메시지 전체**가 깨졌고, src-tauri 수신부는 그 실패를 조용히 버려 증상이
    /// "트리가 영영 안 바뀐다" 뿐이었다.
    // ADR-0172
    #[test]
    fn an_escaped_failure_kind_string_still_decodes() {
        use crate::domain::AgentFailureKind;

        // `O` = 'O' — 값은 같고 인코딩만 이스케이프다.
        let escaped = r#""Other""#;
        assert_eq!(
            serde_json::from_str::<AgentFailureKind>(escaped).expect("이스케이프도 디코드된다"),
            AgentFailureKind::Other
        );

        // 모르는 어휘 + 이스케이프 조합도 흡수로 떨어진다(오류가 아니다).
        let unknown_escaped = r#""SomeKindFromANewerDaemon""#;
        assert_eq!(
            serde_json::from_str::<AgentFailureKind>(unknown_escaped).expect("흡수"),
            AgentFailureKind::Other
        );

        // 소유 값 경로(`from_value`)도 같다 — 빌린 `&str` 은 여기서도 실패했다.
        let owned = serde_json::Value::String("SpawnFailed".to_string());
        assert_eq!(
            serde_json::from_value::<AgentFailureKind>(owned).expect("from_value 경로"),
            AgentFailureKind::SpawnFailed
        );
    }

    // ── 명령 버스 wire 계약(ADR-0155 결정 3 의 2단계) ─────────────────────────────────
    //
    // ★이 golden 들이 지키는 것은 손으로 적은 `#[ts(type = …)]` 이다★ — 봉투 타입은 도구 crate 소유라
    //   ts-rs derive 가 없고(그 crate 가 ts-rs 를 안 든다), 그래서 TS 칸을 이 파일이 손으로 적는다. Rust 쪽
    //   모양이 바뀌어도 그 문자열은 조용히 그대로 남으므로, 실제 JSON 을 여기서 못박아 갈림을 드러낸다
    //   (`CommandDecl` 이 같은 이유로 같은 짝을 갖는다).
    //
    // ★한 헬퍼가 중계 다리와 발신자 다리에 **함께** 쓰이는 것은 의도다★ — 두 다리의 봉투가 같은 타입·같은
    //   모양이라는 것이 계약이고(TRD §3-2 대칭), 다리마다 다른 헬퍼를 두면 그 갈림이 안 보인다.

    fn nil_envelope() -> engram_dashboard_command::CommandEnvelope {
        engram_dashboard_command::CommandEnvelope {
            name: "tab.create".to_string(),
            request_id: engram_dashboard_command::RequestId(Uuid::nil()),
            owner: OwnerToken::new("shell"),
            proto_ver: 1,
            args: serde_json::json!({ "window": "main" }),
        }
    }

    #[test]
    fn command_request_json_golden_and_roundtrip() {
        let ev = AgentEvent::CommandRequest {
            envelope: nil_envelope(),
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert_eq!(
            json,
            r#"{"CommandRequest":{"envelope":{"name":"tab.create","request_id":"00000000-0000-0000-0000-000000000000","owner":"shell","proto_ver":1,"args":{"window":"main"}}}}"#,
            "CommandRequest wire 형태가 golden 과 불일치 — 손으로 적은 TS 칸도 함께 고칠 것"
        );
        let back: AgentEvent = serde_json::from_str(&json).unwrap();
        let AgentEvent::CommandRequest { envelope } = &back else {
            panic!("CommandRequest 로 복호돼야 한다");
        };
        assert_eq!(envelope, &nil_envelope());
    }

    #[test]
    fn command_outcome_json_golden_and_roundtrip() {
        let request_id = engram_dashboard_command::RequestId(Uuid::nil());
        let failed = AgentCommand::CommandOutcome {
            reply: engram_dashboard_command::CommandReply::err(
                request_id,
                engram_dashboard_command::CommandError::of(
                    engram_dashboard_command::ErrorCode::NotFound,
                    "no view 'x'",
                ),
            ),
        };
        let json = serde_json::to_string(&failed).unwrap();
        assert_eq!(
            json,
            r#"{"CommandOutcome":{"reply":{"request_id":"00000000-0000-0000-0000-000000000000","outcome":{"Err":{"code":"NOT_FOUND","message":"no view 'x'","retry":"never"}}}}}"#,
            "실패 결말의 wire 형태가 golden 과 불일치 — 손으로 적은 TS 칸도 함께 고칠 것"
        );
        let back: AgentCommand = serde_json::from_str(&json).unwrap();
        let AgentCommand::CommandOutcome { reply } = &back else {
            panic!("CommandOutcome 으로 복호돼야 한다");
        };
        assert_eq!(reply.request_id, request_id, "상관 키가 왕복을 건넌다");

        let ok = AgentCommand::CommandOutcome {
            reply: engram_dashboard_command::CommandReply::ok(
                request_id,
                serde_json::json!({ "view_id": "v1" }),
            ),
        };
        assert_eq!(
            serde_json::to_string(&ok).unwrap(),
            r#"{"CommandOutcome":{"reply":{"request_id":"00000000-0000-0000-0000-000000000000","outcome":{"Ok":{"view_id":"v1"}}}}}"#,
            "성공 결말도 같은 봉투 모양이어야"
        );
    }

    /// ★손으로 적은 TS 칸이 광고하는 **관용**을 Rust 쪽에 못박는다★.
    ///
    /// 그 칸은 컴파일러가 안 보므로, Rust 가 나중에 조여지면(세 칸을 필수로 만들거나 계약 밖 필드를 거부)
    /// 광고가 조용히 거짓이 된다 — 그때 TS 호출자는 Rust 가 받는 값을 타입으로 표현할 수 없게 된다.
    /// ★위 두 골든은 이 클래스를 못 잡는다★(그것들은 Rust→JSON 한 방향만 본다). 그래서 방향을 뒤집어 잰다.
    #[test]
    fn the_outcome_accepts_everything_its_ts_type_advertises() {
        let nil = "00000000-0000-0000-0000-000000000000";
        // 세 칸 전부 **부재** + 계약 밖 필드.
        let sparse = format!(
            r#"{{"CommandOutcome":{{"reply":{{"request_id":"{nil}","outcome":{{"Err":{{"detail":{{"n":1}}}}}}}}}}}}"#
        );
        let cmd: AgentCommand =
            serde_json::from_str(&sparse).expect("세 칸이 없어도 받는다(TS 칸이 그렇게 광고한다)");
        let AgentCommand::CommandOutcome { reply } = &cmd else {
            panic!("CommandOutcome");
        };
        assert!(reply.outcome.is_err());
        // ★계약 밖 필드는 되보낼 때 살아 있어야 한다★ — additive 확장이 중계 홉에서 증발하지 않는다는 계약.
        assert!(
            serde_json::to_string(&cmd).unwrap().contains("\"detail\""),
            "계약 밖 필드가 중계에서 사라졌다"
        );

        // 세 칸 전부 **`null`**.
        let nulls = format!(
            r#"{{"CommandOutcome":{{"reply":{{"request_id":"{nil}","outcome":{{"Err":{{"code":null,"message":null,"retry":null}}}}}}}}}}"#
        );
        serde_json::from_str::<AgentCommand>(&nulls).expect("null 세 칸도 받는다");
    }

    /// ★중계 다리의 두 variant 는 pending 매칭 밖이다★ — 하나는 들어오는 **요청**이고 하나는 나가는
    /// **답장**이라, 둘 중 어느 쪽이든 `Some` 을 주면 봉투가 삼켜지거나 빈 슬롯이 영구히 남는다.
    #[test]
    fn the_relay_leg_is_outside_pending_correlation() {
        assert_eq!(
            event_reply_request_id(&AgentEvent::CommandRequest {
                envelope: nil_envelope(),
            }),
            None,
            "들어온 요청을 「내가 기다린 답장」으로 읽으면 그 명령은 실행되지 않고 사라진다"
        );
        assert_eq!(
            command_request_id(&AgentCommand::CommandOutcome {
                reply: engram_dashboard_command::CommandReply::ok(
                    engram_dashboard_command::RequestId(Uuid::nil()),
                    serde_json::Value::Null,
                ),
            }),
            None,
            "답장을 보내면서 pending 슬롯을 만들면 깨울 짝이 없다"
        );
    }

    #[test]
    fn command_json_golden_and_roundtrip() {
        let cmd = AgentCommand::Command {
            envelope: nil_envelope(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(
            json,
            r#"{"Command":{"envelope":{"name":"tab.create","request_id":"00000000-0000-0000-0000-000000000000","owner":"shell","proto_ver":1,"args":{"window":"main"}}}}"#,
            "Command wire 형태가 golden 과 불일치 — 손으로 적은 TS 칸도 함께 고칠 것"
        );
        let back: AgentCommand = serde_json::from_str(&json).unwrap();
        let AgentCommand::Command { envelope } = &back else {
            panic!("Command 로 복호돼야 한다");
        };
        assert_eq!(envelope, &nil_envelope());
    }

    #[test]
    fn command_reply_json_golden_and_roundtrip() {
        let request_id = engram_dashboard_command::RequestId(Uuid::nil());
        let failed = AgentEvent::CommandReply {
            reply: engram_dashboard_command::CommandReply::err(
                request_id,
                engram_dashboard_command::CommandError::of(
                    engram_dashboard_command::ErrorCode::NotFound,
                    "no view 'x'",
                ),
            ),
        };
        let json = serde_json::to_string(&failed).unwrap();
        assert_eq!(
            json,
            r#"{"CommandReply":{"reply":{"request_id":"00000000-0000-0000-0000-000000000000","outcome":{"Err":{"code":"NOT_FOUND","message":"no view 'x'","retry":"never"}}}}}"#,
            "실패 답장의 wire 형태가 golden 과 불일치 — 손으로 적은 TS 칸도 함께 고칠 것"
        );
        let back: AgentEvent = serde_json::from_str(&json).unwrap();
        let AgentEvent::CommandReply { reply } = &back else {
            panic!("CommandReply 로 복호돼야 한다");
        };
        assert_eq!(reply.request_id, request_id, "상관 키가 왕복을 건넌다");

        let ok = AgentEvent::CommandReply {
            reply: engram_dashboard_command::CommandReply::ok(
                request_id,
                serde_json::json!({ "view_id": "v1" }),
            ),
        };
        assert_eq!(
            serde_json::to_string(&ok).unwrap(),
            r#"{"CommandReply":{"reply":{"request_id":"00000000-0000-0000-0000-000000000000","outcome":{"Ok":{"view_id":"v1"}}}}}"#,
            "성공 답장도 같은 봉투 모양이어야"
        );
    }

    /// ★발신자 다리의 두 variant 는 **한 쌍으로** pending 매칭 안이다★ — 중계 다리(위)의 정반대다.
    ///
    /// 한쪽만 `Some` 이면 왕복이 안 닫힌다: 요청만 `Some` 이면 슬롯을 깨울 짝이 없고, 답장만 `Some` 이면
    /// 깰 슬롯이 없다. 둘 다 셸이 마감시각을 다 쓰거나 연결이 끊길 때까지 매달리는 것으로 끝난다.
    /// 이 테스트가 그 한쪽만 고친 편집을 떨어뜨린다.
    #[test]
    fn the_sender_leg_is_inside_pending_correlation_as_a_pair() {
        let envelope = engram_dashboard_command::CommandEnvelope {
            request_id: engram_dashboard_command::RequestId::new(),
            ..nil_envelope()
        };
        let sent = AgentCommand::Command {
            envelope: envelope.clone(),
        };
        let answered = AgentEvent::CommandReply {
            reply: engram_dashboard_command::CommandReply::ok(
                envelope.request_id,
                serde_json::json!({ "view_id": "v1" }),
            ),
        };

        let asked =
            command_request_id(&sent).expect("요청이 pending 슬롯을 못 만들면 답장이 와도 못 깬다");
        let woke =
            event_reply_request_id(&answered).expect("답장이 상관 밖이면 그 슬롯을 깨울 짝이 없다");
        assert_eq!(asked, woke, "두 다리가 같은 키를 봐야 왕복이 닫힌다");
        // 홉에서 키를 새로 만들면(`RequestId::new`·`default`) 답장이 이 요청에 못 붙는다.
        assert_eq!(
            asked.0, envelope.request_id.0,
            "봉투가 품은 uuid 그대로여야"
        );
    }
}
