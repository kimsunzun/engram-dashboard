//! wire 메시지 — UI→core [`AgentCommand`], core→UI [`AgentEvent`].
//! 둘 다 externally-tagged JSON(serde 기본).

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
