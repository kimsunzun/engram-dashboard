//! ID 타입. 모두 Uuid 기반(현 core 와 동일). ts-rs 로 TS string 에 매핑.

use ts_rs::TS;

/// 에이전트 고유 식별자. core 의 `AgentId = uuid::Uuid` 와 동일 표현.
pub type AgentId = uuid::Uuid;

/// 영속 프로필 식별자(Spawn 요청이 참조). 현 core 의 profile id 와 동일.
pub type ProfileId = uuid::Uuid;

/// 프리셋 식별자(DeletePreset 요청이 참조). core `preset::PresetId` 와 동일 표현(ADR-0061).
pub type PresetId = uuid::Uuid;

/// side-effect command 의 idempotency 키(설계 §3). 데몬이 짧은 TTL dedup table 로 중복 흡수.
/// 자동 재시도 금지(writeStdin 중복=입력 중복) — 끊김 시 reconnect 후 결과 조회.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct RequestId(#[ts(type = "string")] pub uuid::Uuid);

impl RequestId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl Default for RequestId {
    fn default() -> Self {
        Self::new()
    }
}
