//! 도메인 타입(wire 표현). 현 `core::agent::types` / `core::agent::profile` 의 직렬화 형태를 미러.
//! phase 1 에서 core 가 이 crate 에 의존하며 단일 진실원으로 합쳐진다(중복 제거).

use ts_rs::TS;

use crate::ids::{AgentId, PresetId, ProfileId};

/// 에이전트 생명주기 상태 — internally-tagged(`type`). 프론트 discriminated union.
/// core(types.rs) AgentStatus 와 글자 그대로 일치.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
pub enum AgentStatus {
    Running,
    Exiting,
    Exited { code: Option<i32> },
    Failed { message: String },
    Killed,
}

/// 영역별 capability(bool 폭증 방지). 슬롯이 렌더러/UI 분기에 사용.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct Capabilities {
    pub input: InputCaps,
    pub output: OutputCaps,
    pub control: ControlCaps,
    pub session: SessionCaps,
    pub model: ModelCaps,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct InputCaps {
    pub raw: bool,
    pub message: bool,
    pub attachment: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct OutputCaps {
    pub terminal_bytes: bool,
    /// 구조화 스트림(NDJSON) 여부 — M2 렌더러 분기(xterm vs RichSlot)를 위한 신호(ADR-0044). core 미러.
    /// `#[serde(default)]`(FIX 3): M1 에서 새로 추가된 필드라, 이 필드가 없는 옛 wire(구 데몬/프론트)를
    /// 받아도 관용적으로 false 로 역직렬화한다(sibling `output_format` 과 같은 additive·tolerant 접근 —
    /// PROTOCOL_VERSION 유지). ts-rs 는 serde(default) 를 optional 로 표기하지 않으므로 TS 는 여전히
    /// `structured: boolean`(non-optional) — 프론트는 손댈 필요 없다.
    #[serde(default)]
    pub structured: bool,
    pub markdown: bool,
    pub tool_events: bool,
    pub usage: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct ControlCaps {
    pub resize: bool,
    pub interrupt: bool,
    pub cancel: bool,
    pub graceful_shutdown: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct SessionCaps {
    pub resume: bool,
    pub snapshot: bool,
    pub cwd_env: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct ModelCaps {
    pub select: bool,
    pub temperature: bool,
    pub max_tokens: bool,
}

/// 에이전트 메타데이터 스냅샷 — AgentListUpdated 및 연결 직후 list 스냅샷에 실림.
/// core(types.rs) AgentInfo 와 일치. epoch 는 재구독 트리거(`[agentId,epoch]`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct AgentInfo {
    #[ts(type = "string")]
    pub id: AgentId,
    /// 표시용 이름(ProfileRegistry 단일 진실원, 없으면 id 앞 8자).
    pub name: String,
    pub cwd: String,
    pub status: AgentStatus,
    pub cols: u16,
    pub rows: u16,
    pub epoch: u32,
    pub capabilities: Capabilities,
}

/// 복원 결말 — core(profile.rs) RestoreOutcome 미러(`type` tag).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
pub enum RestoreOutcome {
    Resumed,
    Started,
    FreshFallback {
        old_sid: Option<String>,
        new_sid: String,
        reason: String,
    },
    Blocked {
        reason: String,
    },
    Failed {
        reason: String,
    },
}

/// 복원 시도 결과 통지(AgentEvent::RestoreResult 페이로드).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct RestoreReport {
    #[ts(type = "string")]
    pub agent_id: AgentId,
    pub epoch: u32,
    pub outcome: RestoreOutcome,
}

// ── 프로필 wire 미러(phase4 1단계) ──────────────────────────────────────────────
//
// core(profile.rs) 의 AgentProfile/AgentCommand/RestartPolicy 직렬화 형태를 그대로 미러한다.
// core 는 protocol 무의존(§1 불변)이라 core 타입을 여기 쓸 수 없다 — 그래서 같은 JSON 형태의
// 독립 타입을 두고, core↔wire 명시 변환은 데몬이 한다(reflection 왕복 금지 — agent_info_to_wire 패턴).
// 프론트 `src/api/types.ts` 의 AgentProfile/AgentCommand/RestartPolicy 와 글자 그대로 일치.

/// claude 출력 포맷 wire 미러 — core `profile::ClaudeOutputFormat` 와 동일(ADR-0044).
/// Terminal=PTY 대화형, StreamJson=헤드리스 NDJSON. 프론트 `src/api/types.ts` 와 글자 그대로 일치.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub enum ClaudeOutputFormat {
    #[default]
    Terminal,
    StreamJson,
}

/// 에이전트 실행 명령 wire 미러 — core `profile::AgentCommand` 와 동일(`#[serde(tag="kind")]`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum AgentSpawnCommand {
    /// claude CLI. extra_args 는 세션 인자를 제외한 사용자 추가 인자.
    /// output_format 은 터미널/JSON 모드(ADR-0044) — `#[serde(default)]` 라 옛 프로필은 Terminal.
    Claude {
        extra_args: Vec<String>,
        #[serde(default)]
        output_format: ClaudeOutputFormat,
    },
    /// 임의 셸 프로그램.
    Shell { program: String, args: Vec<String> },
}

/// 자동 재시작 정책 wire 미러 — core `profile::RestartPolicy` 와 동일.
/// **예약(reserved) — 죽은 필드 아님.** 동작 미구현이나 ADR-0016 "추후 재검토" 유효(2026-06-18 결정).
/// 제거 시 core·ts-rs 바인딩·프론트 동반 + PROTOCOL_VERSION bump 유발 → 제거 금지.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub enum RestartPolicy {
    Never,
    OnCrash,
    Always,
}

/// 영속 프로필 wire 미러 — core `profile::AgentProfile` 의 직렬화 형태와 일치.
/// 프로필 CRUD command/event(ProfileListUpdated)에 실린다.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct AgentProfile {
    #[ts(type = "string")]
    pub id: ProfileId,
    pub name: String,
    pub command: AgentSpawnCommand,
    /// 정규화된 cwd(PathBuf 의 JSON 표현 = 문자열).
    pub cwd: String,
    /// ※자격증명 금지(평문 persist).
    pub env: Vec<(String, String)>,
    /// 현재 claude 세션 id(없으면 None).
    #[ts(type = "string | null")]
    pub claude_session_id: Option<String>,
    /// 폐기된 과거 세션 id 이력.
    #[ts(type = "string[]")]
    pub old_session_ids: Vec<String>,
    pub epoch: u32,
    pub auto_restore: bool,
    /// **예약(reserved)** — 동작 미구현, 제거 금지(RestartPolicy 주석 참조).
    pub restart_policy: RestartPolicy,
    /// 크래시 가드 카운터(수동 재시작 시 0 리셋). **예약(reserved)** — 동작 미구현, ADR-0016 유효.
    pub restart_count: u32,
    /// Failed(자동복원 suspend) 사유 — 콜드부팅 넘어 영속, 수동 깨우기 전까지 자동복원 제외(ADR-0016).
    /// **예약(reserved)** — 동작 미구현이나 ADR-0016에서 유효, 제거 금지(버전 bump 유발).
    #[ts(type = "string | null")]
    pub failed_reason: Option<String>,
    pub created_at: i64,
    pub last_active: i64,
    /// 마지막 프로세스 기동 시각(기록·디버깅용, 리셋 판정엔 미사용).
    #[ts(type = "number | null")]
    pub last_start_at: Option<i64>,
}

// ── 프리셋 wire 미러(ADR-0061) ──────────────────────────────────────────────────
//
// core(preset.rs) 의 Preset 직렬화 형태를 그대로 미러한다. core 는 protocol 무의존(§1 불변)이라
// core 타입을 여기 쓸 수 없다 — 같은 JSON 형태의 독립 타입을 두고, core↔wire 명시 변환은 데몬이
// 한다(profile_to_wire 패턴 동일). 프로필과 1:1 대응하는 최소 스키마 `{ id, cwd }`(이름 파생 — ADR-0061).

/// 영속 프리셋 wire 미러 — core `preset::Preset` 의 직렬화 형태와 일치(ADR-0061). 프리셋 CRUD
/// command/event(PresetList/PresetListUpdated)에 실린다. cwd 는 PathBuf 의 JSON 표현(문자열).
/// 이름은 저장하지 않고 프론트가 cwd basename 으로 파생한다(ADR-0061).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct Preset {
    #[ts(type = "string")]
    pub id: PresetId,
    /// 정규화된 cwd(PathBuf 의 JSON 표현 = 문자열).
    pub cwd: String,
}

/// 출력 스냅샷 청크 wire 미러 — core `types::OutputChunk`({seq, data}) 와 일치.
/// GetSnapshot 응답(AgentEvent::Snapshot)에 실린다. JSON 경로라 data 는 number[].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct SnapshotChunk {
    #[ts(type = "number")]
    pub seq: u64,
    #[serde(with = "serde_bytes")]
    #[ts(type = "number[]")]
    pub data: Vec<u8>,
}
