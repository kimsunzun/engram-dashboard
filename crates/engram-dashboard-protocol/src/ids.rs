use ts_rs::TS;

/// Same representation as agent's `AgentId = uuid::Uuid`.
pub type AgentId = uuid::Uuid;

/// Same as the profile id in the current agent crate.
pub type ProfileId = uuid::Uuid;

/// Same representation as agent `preset::PresetId` (ADR-0061).
pub type PresetId = uuid::Uuid;

/// Idempotency key of a side-effect command (design §3). The daemon absorbs duplicates with a short-TTL dedup table.
/// No automatic retry (duplicate writeStdin = duplicate input) — on disconnect, query the result after reconnecting.
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

/// The owner mark of registration packets (`RegisterCommands` · `UpdateCommands`) — the wire copy of the same-named command bus tool crate type (ADR-0310 decision 23).
///
/// - Serialization = the inner string as is (`serde(transparent)`).
/// - Conversion into the tool crate type happens only at the daemon edge (`owner_from_wire` in `connection_core.rs`), so protocol does not depend on the tool crate.
/// - ★Do not attach `TS`★: only while this type has no `TS` does removing the field's `#[ts(type = "string")]` break compilation, which keeps `owner: string` in
///   `AgentCommand.ts`; with `TS` and no override the generated file silently becomes `owner: OwnerToken`.
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
