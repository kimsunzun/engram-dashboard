//! wire 메시지 — UI→core [`AgentCommand`], core→UI [`AgentEvent`].
//! 둘 다 externally-tagged JSON(serde 기본). 단 고-throughput TerminalBytes 출력은
//! JSON 이 아닌 binary frame(`codec`)으로 흐른다(설계 §1-2).

use ts_rs::TS;

use crate::domain::{
    AgentInfo, AgentProfile, AgentStatus, Capabilities, ClaudeOutputFormat, Preset, RestoreReport,
    SnapshotChunk,
};
use crate::ids::{AgentId, PresetId, ProfileId, RequestId};

/// UI→core 요청 envelope(설계 §3). side-effect 명령은 `request_id` 로 idempotent.
/// (Profile CRUD 는 phase 1 에서 core profile 타입 합류 후 추가 — 지금은 보류.)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub enum AgentCommand {
    /// 연결 후 첫 frame 전용 인증(설계 §4 step 4b). 데몬이 "연결 1초 내 첫 frame"으로만 유효성을
    /// 강제한다 — 그 외 시점의 Auth 는 무시한다. token 은 daemon.json 의 256-bit hex.
    Auth {
        token: String,
        protocol_version: u32,
    },
    /// 새 에이전트 spawn. 프로필 참조.
    Spawn {
        #[ts(type = "string")]
        profile_id: ProfileId,
        request_id: RequestId,
    },
    /// 에이전트 종료(자원 강제 폐쇄).
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
    /// stdin 입력 전달. raw 바이트(키 입력). idempotency 키 필수(중복=입력 중복).
    WriteStdin {
        #[ts(type = "string")]
        agent_id: AgentId,
        #[serde(with = "serde_bytes")]
        #[ts(type = "number[]")]
        data: Vec<u8>,
        request_id: RequestId,
    },
    /// PTY 크기 변경. viewport_id 는 멀티뷰 중 어느 뷰가 요청했는지(ControlLease 판정용).
    Resize {
        #[ts(type = "string")]
        agent_id: AgentId,
        cols: u16,
        rows: u16,
        viewport_id: Option<String>,
    },
    /// 출력 구독. epoch/after_seq 로 재연결 resume(설계 §1-3).
    /// 둘 다 None = 처음부터(oldest 부터) 받겠다는 신규 구독.
    Subscribe {
        #[ts(type = "string")]
        agent_id: AgentId,
        epoch: Option<u32>,
        #[ts(type = "number | null")]
        after_seq: Option<u64>,
    },
    /// 구독 해제.
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
    /// 입력 lease 해제. 보유자만 해제할 수 있다(보유자 아니면 Error). 해제 후엔 누구나 다시 acquire 가능.
    ReleaseInput {
        #[ts(type = "string")]
        agent_id: AgentId,
        request_id: RequestId,
    },
    /// 전체 에이전트 목록 조회(연결 직후 데몬이 자동 push 도 하지만 명시 조회도 허용).
    /// 응답은 request_id 동봉 [`AgentEvent::AgentList`](전용 reply). broadcast 인
    /// [`AgentEvent::AgentListUpdated`](트리 실시간 갱신)와 별개 — 편승 매칭 제거.
    ListAgents { request_id: RequestId },
    /// 데몬 종료(§5 LLM 제어). force=true 면 활성 에이전트 있어도 종료, kill_agents=true 면 함께 정리.
    StopDaemon {
        force: bool,
        kill_agents: bool,
        request_id: RequestId,
    },

    // ── 프로필 CRUD + ad-hoc spawn(phase4 1단계) ───────────────────────────────────
    // EmbeddedClient(invoke)의 프로필 메서드와 1:1 대응. 두 모드가 같은 동작을 해야
    // DaemonClient 가 EmbeddedClient 와 호환된다(아래 각 variant 주석에 대응 invoke 명시).
    /// cwd 만으로 ad-hoc 셸 에이전트 spawn(영속 프로필 없이 transient). EmbeddedClient `spawnAgent(cwd)`
    /// = Tauri `spawn_agent` 대응 — 기본 셸 명령 + auto_restore=false 로 Fresh spawn.
    SpawnByCwd { cwd: String, request_id: RequestId },

    /// 저장된 프로필 전체 조회. EmbeddedClient `listProfiles` = Tauri `list_profiles` 대응.
    /// 응답은 request_id 동봉 [`AgentEvent::ProfileList`](전용 reply). broadcast 인
    /// [`AgentEvent::ProfileListUpdated`](프론트 미러 갱신)와 별개 — 편승 매칭 제거.
    ListProfiles { request_id: RequestId },

    /// claude 프로필 생성(스폰하지 않음 — 등록·persist만). EmbeddedClient `createClaudeProfile`
    /// = Tauri `create_claude_profile` 대응. ※env 에 자격증명 금지(평문 persist).
    CreateProfile {
        name: String,
        cwd: String,
        extra_args: Vec<String>,
        env: Vec<(String, String)>,
        auto_restore: bool,
        /// claude 출력 포맷(ADR-0044 M2) — Terminal=PTY 대화형(기본) / StreamJson=헤드리스 NDJSON.
        /// `#[serde(default)]` 라 이 필드 없는 옛 프론트/wire 는 Terminal 로 흡수(기존 동작 불변,
        /// PROTOCOL_VERSION 유지 — sibling OutputCaps.structured 와 같은 additive·tolerant 접근).
        /// 데몬이 이 값을 저장 프로필의 AgentCommand::Claude { output_format } 로 옮기고, 이후
        /// SpawnProfile → manager.spawn_agent 가 is_json_mode 로 StdioTransport 를 고른다.
        #[serde(default)]
        output_format: ClaudeOutputFormat,
        request_id: RequestId,
    },

    /// 프로필 삭제. EmbeddedClient `deleteProfile` = Tauri `delete_profile` 대응.
    DeleteProfile {
        #[ts(type = "string")]
        profile_id: ProfileId,
        request_id: RequestId,
    },

    /// 저장된 프로필 spawn. resume=true 면 기존 세션 이어받기(claude `--resume`).
    /// EmbeddedClient `spawnProfile(agentId, resume)` = Tauri `spawn_profile` 대응.
    SpawnProfile {
        #[ts(type = "string")]
        profile_id: ProfileId,
        resume: bool,
        request_id: RequestId,
    },

    /// auto_restore 토글. EmbeddedClient `setProfileAutoRestore` = Tauri `set_profile_auto_restore` 대응.
    SetProfileAutoRestore {
        #[ts(type = "string")]
        profile_id: ProfileId,
        auto_restore: bool,
        request_id: RequestId,
    },

    /// 프로필 표시명 override 설정/해제(ADR-0061 리치화 — 트리 rename). `name=Some` → override 저장,
    /// `None` → 해제(cwd basename 파생 복귀). trim·빈문자열 거부·미변경 스킵은 프론트가 확정 직전 처리
    /// (TabBar rename 과 동형) — 여기엔 유효 값 또는 명시 None 만 온다. 없는 id 면 Error(SetProfileAutoRestore
    /// 와 동형). 성공 후 [`AgentEvent::ProfileListUpdated`] broadcast(낙관 갱신 X — 모든 창 동기화).
    RenameProfile {
        #[ts(type = "string")]
        profile_id: ProfileId,
        #[ts(type = "string | null")]
        name: Option<String>,
        request_id: RequestId,
    },

    /// replay buffer 스냅샷 조회. EmbeddedClient `getSnapshot` = Tauri `get_agent_snapshot` 대응.
    /// 응답은 [`AgentEvent::Snapshot`]. (Subscribe replay 와 별개의 1회성 조회.)
    GetSnapshot {
        #[ts(type = "string")]
        agent_id: AgentId,
        request_id: RequestId,
    },

    // ── 프리셋 CRUD(ADR-0061) ──────────────────────────────────────────────────────
    // 프로필 CRUD(ListProfiles/CreateProfile/DeleteProfile)와 1:1 대응하는 프리셋판. 프리셋 =
    // 스폰 전 "cwd 북마크"(인스턴스 아님). 데몬이 presets.json 을 단일 소유하고 wire 로만 CRUD 한다.
    /// 저장된 프리셋 전체 조회. 응답은 request_id 동봉 전용 reply [`AgentEvent::PresetList`]
    /// (요청 연결에만). broadcast 인 [`AgentEvent::PresetListUpdated`](CRUD 후)와 별개 — 편승 매칭 제거.
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
}

/// core→UI 이벤트 envelope(설계 §3, JSON 경로). TerminalBytes 출력은 여기 없음(binary frame).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub enum AgentEvent {
    /// 연결 직후 핸드셰이크. 버전·capability 통보.
    Hello {
        protocol_version: u32,
        daemon_version: String,
        /// 데몬 전체 capability(에이전트별 capability 는 AgentInfo 에).
        capabilities: Option<Capabilities>,
    },
    /// side-effect command 수신/처리 확인(request_id 에코).
    Ack { request_id: RequestId },
    /// Subscribe 응답 — replay 방식과 범위(설계 §1-3).
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
        /// ring 밖으로 밀려 일부 손실(clear+tail). UI "output truncated" 표시.
        truncated: bool,
    },
    /// 저빈도 구조화 출력(TextDelta/Usage/ToolCall 등). TerminalBytes 는 binary frame 으로 감.
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
    /// 상태 변경. epoch 동봉(옛 세션 stale 알림 방어).
    StatusChanged {
        #[ts(type = "string")]
        agent_id: AgentId,
        status: AgentStatus,
        epoch: u32,
    },
    /// 전체 목록 갱신(broadcast). terminal 판정은 이걸로(status_changed 아님 — 설계 불변식).
    /// ※ 트리 실시간 갱신 전용 — request_id 없음. ListAgents 조회 응답은 [`AgentEvent::AgentList`].
    AgentListUpdated { agents: Vec<AgentInfo> },
    /// ListAgents 조회 응답(전용 reply) — request_id 에코로 "내 요청 결과"를 정확히 매칭.
    /// broadcast 인 AgentListUpdated 와 페이로드는 동일하나 편승 매칭(다음 도착 메시지 짝짓기)을
    /// 제거하기 위해 request_id 를 동봉한다(Spawned/Created 와 동형).
    AgentList {
        request_id: RequestId,
        agents: Vec<AgentInfo>,
    },
    /// 복원 시도 결과.
    RestoreResult { report: RestoreReport },
    /// 입력 lease 상태 변경 통보(다중 뷰어가 "지금 잠겨있음"을 알게 함). held=true 면 누군가 보유 중,
    /// false 면 비어 있음(아무나 acquire 가능). 보유자 conn 식별값은 보안상 노출하지 않는다(잠김 여부만).
    InputLeaseChanged {
        #[ts(type = "string")]
        agent_id: AgentId,
        held: bool,
    },
    /// 프로필 목록 갱신(broadcast, phase4 1단계). CRUD(생성/삭제/토글) 후 자동 push — 프론트
    /// ProfileRegistry 미러 갱신용. AgentListUpdated 의 프로필판. request_id 없음.
    /// ListProfiles 조회 응답은 [`AgentEvent::ProfileList`].
    ProfileListUpdated { profiles: Vec<AgentProfile> },
    /// ListProfiles 조회 응답(전용 reply) — request_id 에코. broadcast 인 ProfileListUpdated 와
    /// 페이로드는 같으나 편승 매칭 제거를 위해 request_id 동봉(Spawned/Created 와 동형).
    ProfileList {
        request_id: RequestId,
        profiles: Vec<AgentProfile>,
    },

    /// 프리셋 목록 갱신(broadcast, ADR-0061). CRUD(생성/삭제) 후 자동 push — 모든 창의 프리셋 미러
    /// 동기화용. ProfileListUpdated 의 프리셋판. request_id 없음. ListPresets 조회 응답은 [`AgentEvent::PresetList`].
    PresetListUpdated { presets: Vec<Preset> },
    /// ListPresets 조회 응답(전용 reply, ADR-0061) — request_id 에코. broadcast 인 PresetListUpdated 와
    /// 페이로드는 같으나 편승 매칭 제거를 위해 request_id 동봉(ProfileList 와 동형).
    PresetList {
        request_id: RequestId,
        presets: Vec<Preset>,
    },

    /// GetSnapshot 응답(전용 reply, phase4 1단계) — 그 시점 replay buffer 스냅샷.
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

    /// 오류 통지. request_id 있으면 특정 command 실패.
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
/// ★TerminalBytes 는 제외★: 콘솔 raw 바이트는 tag0 terminal frame(payload=raw bytes)으로만 흐르고 tag1
/// payload 에 실리지 않는다(codec.rs: tag0=TerminalBytes / tag1=StructuredEvent). 따라서 이 미러에는
/// TerminalBytes variant 를 두지 않는다 — core `OutputEvent::TerminalBytes` 가 이 변환에 오면 adapter 가
/// 방어적으로 흡수(근거 주석은 output_event_to_wire).
///
/// ★self-describing serde★: internally-tagged(`#[serde(tag="type")]`) — payload JSON 에 `"type"` 판별자가
/// 박혀 프론트가 JSON.parse 후 variant 를 가른다(codec 은 이 스키마를 모른다 — opaque tag1 payload, ADR-0045).
/// wire 직렬화 형식 = JSON(serde_json) — daemon adapter 가 `serde_json::to_vec` 로 tag1 payload 를 만든다.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
pub enum StructuredEvent {
    /// 어시스턴트 텍스트 증분(스트리밍 델타). core `OutputEvent::TextDelta` 미러.
    TextDelta {
        text: String,
        turn_id: Option<String>,
        message_id: Option<String>,
    },
    /// 도구 호출 — 이름 + 직렬화된 인자(backend별 스키마 그대로). core `OutputEvent::ToolCall` 미러.
    ToolCall {
        name: String,
        args_json: String,
        /// 호출 식별자(권한 UX·결과 매칭용). claude tool_use id 등.
        id: Option<String>,
        turn_id: Option<String>,
        message_id: Option<String>,
    },
    /// 토큰 사용량. core `OutputEvent::Usage` 미러.
    Usage {
        #[ts(type = "number")]
        input_tokens: u64,
        #[ts(type = "number")]
        output_tokens: u64,
        turn_id: Option<String>,
    },
    /// 한 메시지(turn 응답) 종료 신호. core `OutputEvent::MessageDone` 미러.
    MessageDone {
        turn_id: Option<String>,
        message_id: Option<String>,
    },
    /// backend 가 보고한 오류(스트림 내부 오류 — 종료 아님). core `OutputEvent::Error` 미러.
    Error { message: String },
    /// 위 정형 variant 로 안 잡히는 backend별 이벤트의 탈출구(forward-compat). core `OutputEvent::Structured`
    /// 미러 — kind=종류 태그, json=원본 직렬화 payload(프론트가 kind 로 분기·해석).
    Structured { kind: String, json: String },
}

/// 출력 청크 — 종류 불가지(설계 §2). TerminalBytes 는 binary frame(codec)으로,
/// 나머지 구조화 variant 는 JSON(AgentEvent::Output)으로 흐른다.
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
    /// API/구조화 텍스트 증분.
    TextDelta(String),
    /// 토큰 사용량.
    Usage {
        input_tokens: u64,
        output_tokens: u64,
    },
    /// 도구 호출(이름+직렬화 인자).
    ToolCall { name: String, args_json: String },
    /// 임의 구조화 페이로드(forward-compat 탈출구).
    Structured { kind: String, json: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    /// ★ADR-0045★: StructuredEvent 는 core `OutputEvent` 미러이자 tag1 payload 다. self-describing
    /// (`#[serde(tag="type")]`) JSON 직렬화가 무손실 round-trip 되는지(필드 유실 0 — 교체성 핵심) +
    /// `"type"` 판별자가 payload 에 박히는지(프론트 variant 판별 근거) 검증한다. daemon adapter 가
    /// `serde_json::to_vec` 로 만든 tag1 payload 를 프론트가 그대로 JSON.parse 하므로 이 계약이 wire 계약이다.
    #[test]
    fn structured_event_roundtrip_all_variants() {
        let cases = vec![
            StructuredEvent::TextDelta {
                text: "hello".into(),
                turn_id: Some("t1".into()),
                message_id: None, // optional 보존(None 도 왕복)
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
            // self-describing: "type" 판별자가 반드시 박힌다(프론트 variant 판별 근거).
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
    // AgentCommand/AgentEvent 는 externally-tagged(serde 기본) JSON envelope 다. 프리셋 CRUD 가
    // 프로필과 동형 형태(variant 이름 태그 + 필드)로 직렬화되는지 고정한다 — wire 포맷이 조용히
    // 바뀌면(필드 개명/누락) 프론트 미러가 깨지므로 golden 문자열로 회귀를 막는다.

    #[test]
    fn create_preset_command_json_golden() {
        let request_id = RequestId(Uuid::nil());
        let cmd = AgentCommand::CreatePreset {
            cwd: "C:/proj".into(),
            request_id,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        // externally-tagged: variant 이름이 최상위 키.
        assert_eq!(
            json,
            r#"{"CreatePreset":{"cwd":"C:/proj","request_id":"00000000-0000-0000-0000-000000000000"}}"#,
            "CreatePreset wire 형태가 golden 과 불일치"
        );
        // round-trip 무손실.
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
            // 재직렬화가 동일해야(round-trip 무손실).
            assert_eq!(json, serde_json::to_string(&back).unwrap());
        }
    }

    #[test]
    fn preset_list_events_json_golden_and_roundtrip() {
        // name=None(override 없음 — 신규 필드는 null 로 직렬화, ADR-0061 리치화).
        let preset = Preset {
            id: Uuid::nil(),
            cwd: "C:/proj".into(),
            name: None,
        };
        // PresetList(전용 reply — request_id 동봉).
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

        // PresetListUpdated(broadcast — request_id 없음).
        let updated = AgentEvent::PresetListUpdated {
            presets: vec![preset],
        };
        let updated_json = serde_json::to_string(&updated).unwrap();
        assert_eq!(
            updated_json,
            r#"{"PresetListUpdated":{"presets":[{"id":"00000000-0000-0000-0000-000000000000","cwd":"C:/proj","name":null}]}}"#,
            "PresetListUpdated wire 형태가 golden 과 불일치"
        );

        // 둘 다 round-trip 무손실.
        for json in [list_json, updated_json] {
            let back: AgentEvent = serde_json::from_str(&json).unwrap();
            assert_eq!(json, serde_json::to_string(&back).unwrap());
        }
    }

    /// ADR-0061 리치화: RenamePreset wire 형태 golden(externally-tagged) + round-trip. name=Some 케이스로
    /// 실제 override 값이 전달되는 형태를 고정한다(필드 개명/누락 회귀 차단).
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

        // name=None(override 해제) round-trip 무손실.
        let clear = AgentCommand::RenamePreset {
            preset_id: Uuid::nil(),
            name: None,
            request_id: RequestId(Uuid::nil()),
        };
        let clear_json = serde_json::to_string(&clear).unwrap();
        let clear_back: AgentCommand = serde_json::from_str(&clear_json).unwrap();
        assert_eq!(clear_json, serde_json::to_string(&clear_back).unwrap());
    }

    /// ADR-0061 리치화: RenameProfile(트리 rename) wire golden + round-trip. SetProfileAutoRestore 와
    /// 동형 형태(profile_id + 값 + request_id)를 고정한다.
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
}
