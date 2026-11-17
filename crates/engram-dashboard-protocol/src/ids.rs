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

/// The owner mark of registration packets (`RegisterCommands` · `UpdateCommands`) — the wire copy of the same-named type of the command bus tool crate
/// (ADR-0310 decision 23).
///
/// - **Serialization = the inner string as is** (`serde(transparent)`) — the wire bytes are the same as the tool crate type.
/// - Moving it into the tool crate type is done only at the daemon edge (`owner_from_wire` in `connection_core.rs`) —
///   the shell builds the advertised value with this type from the start. Putting the conversion here would keep protocol from dropping the tool crate dependency.
/// - ★Do not attach `TS`★ — only while this type has no `TS` does removing the field's `#[ts(type = "string")]` break compilation,
///   which keeps `owner: string` in `AgentCommand.ts`. If `TS` is attached and that override removed, the generated file
///   silently changes to `owner: OwnerToken` and its import.
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
