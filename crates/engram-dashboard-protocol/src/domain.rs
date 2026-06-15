//! 도메인 타입(wire 표현). 현 `core::agent::types` / `core::agent::profile` 의 직렬화 형태를 미러.
//! phase 1 에서 core 가 이 crate 에 의존하며 단일 진실원으로 합쳐진다(중복 제거).

use ts_rs::TS;

use crate::ids::{AgentId, ProfileId};

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

/// 에이전트 실행 명령 wire 미러 — core `profile::AgentCommand` 와 동일(`#[serde(tag="kind")]`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum AgentSpawnCommand {
    /// claude CLI. extra_args 는 세션 인자를 제외한 사용자 추가 인자.
    Claude { extra_args: Vec<String> },
    /// 임의 셸 프로그램.
    Shell { program: String, args: Vec<String> },
}

/// 자동 재시작 정책 wire 미러 — core `profile::RestartPolicy` 와 동일.
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
    pub restart_policy: RestartPolicy,
    pub created_at: i64,
    pub last_active: i64,
    #[ts(type = "number | null")]
    pub last_restore: Option<i64>,
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
