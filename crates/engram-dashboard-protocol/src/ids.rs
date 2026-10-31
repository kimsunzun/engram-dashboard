use ts_rs::TS;

/// agent 의 `AgentId = uuid::Uuid` 와 동일 표현.
pub type AgentId = uuid::Uuid;

/// 현 agent 의 profile id 와 동일.
pub type ProfileId = uuid::Uuid;

/// agent `preset::PresetId` 와 동일 표현(ADR-0061).
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

/// 등록 패킷(`RegisterCommands` · `UpdateCommands`)의 주인 표지 — 명령 버스 도구 crate 의 같은 이름
/// 타입의 wire 사본이다(ADR-0310 결정 23).
///
/// - **직렬화 = 안쪽 문자열 그대로**(`serde(transparent)`) — 도구 crate 타입과 wire 바이트가 같다.
/// - 도구 crate 타입으로 옮겨 담는 것은 데몬 끝자락 하나다(`connection_core.rs` 의 `owner_from_wire`) —
///   셸은 광고 값을 처음부터 이 타입으로 짓는다. 변환을 여기 두면 protocol 이 도구 crate 의존을 못 놓는다.
/// - ★`TS` 를 달지 않는다★ — 이 타입에 `TS` 가 없어야 칸의 `#[ts(type = "string")]` 를 지우면 컴파일이
///   깨져 `AgentCommand.ts` 의 `owner: string` 이 지켜진다. `TS` 를 달고 그 덮어쓰기를 지우면 생성물이
///   말없이 `owner: OwnerToken` 과 그 import 로 바뀐다.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct OwnerToken(pub String);

impl OwnerToken {
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for OwnerToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
