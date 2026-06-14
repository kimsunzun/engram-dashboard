//! 도메인 타입(wire 표현). 현 `core::pty::types` / `core::pty::profile` 의 직렬화 형태를 미러.
//! phase 1 에서 core 가 이 crate 에 의존하며 단일 진실원으로 합쳐진다(중복 제거).

use ts_rs::TS;

use crate::ids::AgentId;

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
