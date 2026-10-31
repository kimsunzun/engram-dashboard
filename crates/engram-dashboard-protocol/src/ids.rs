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

/// 봉투가 품은 상관 키를 이 crate 의 상관 키로 옮긴다 — **uuid 를 그대로 나른다**(재생성 없음).
///
/// ★거처가 여기인 이유★: 화살표가 `protocol → command` 한 방향이라 도구 crate 는 이 타입을 모른다
/// (ADR-0155 · TRD §3-1). 그쪽에 두려면 역방향 의존이 생겨 「워크스페이스 의존 0」이 깨진다.
/// ★새 값을 만들지 않는 것이 계약이다★ — 상관 키는 왕복 전 구간 동일해야 하고(ADR-0081 결정 4),
/// 홉을 건널 때 새 uuid 가 나면 답장이 어느 요청의 것인지 그 자리에서 끊긴다.
impl From<engram_dashboard_command::RequestId> for RequestId {
    fn from(id: engram_dashboard_command::RequestId) -> Self {
        Self(id.0)
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
