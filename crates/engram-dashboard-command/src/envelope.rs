//! Symmetric envelope — the same shape at every hop (TRD §3-2).

use std::fmt;

use crate::CommandError;

/// Round-trip correlation key — **identical across the whole path** (not created anew at each hop, ADR-0081 decision 4).
///
/// Its representation is **the same** as the same-named type in `engram-dashboard-protocol` (uuid) — a conversion at a hop could break
///   correlation.
/// ★Do not attach `Default` — a request id has no default★: deriving it yields the nil UUID, so **every request has the same correlation
///   key** and a reply attaches to someone else's round trip. The protocol-side type's `default()` mints a fresh v4,
/// the exact opposite, so copying by shape alone would slip that difference in (`#[serde(default)]` and `..Default::default()` reach it
///   when the envelope is carried as an `AgentCommand` variant). The only way to make one is [`RequestId::new`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct RequestId(pub uuid::Uuid);

impl RequestId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The **runtime** owner identifier obtained through registration.
///
/// ★This is not the declared tier★ — the 「owner」 is the crate where the declaration lives, and this token points at which connection that owner
/// is attached to right now. Reading the two as the same thing turns the registry back into a build list (TRD §3-7 prohibition clause).
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct OwnerToken(pub String);

impl OwnerToken {
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OwnerToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Request envelope.
///
/// ★The missing direction field is intentional★ — **the connection it was written to is the direction**; a field would give the envelope
///   two truths and routing splits when they disagree (TRD §3-2).
/// ★`name` is the outer cover, `args` the inside★ — the daemon reads only the cover for per-command authorization and observation, never
///   the inside (ADR-0081 "daemon opaque").
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CommandEnvelope {
    pub name: String,
    pub request_id: RequestId,
    /// ★The **destination** owner token — not the sender★. The previous hop overwrites it from its own registry (`route`), so the
    ///   receiving hop checks "who was this addressed to" against its own registry to sort it
    /// (two-stage delivery, TRD §3-8). ★Not an identity★ — authorization material (TRD §4-⑧) is `name` and **the connection the envelope
    ///   came from**.
    pub owner: OwnerToken,
    /// The `CATALOG_VERSION` of the sending crate. ★For diagnostics, not material for branching★ — generation numbers are per crate, so
    /// if the receiver compared it with its own number and gave it meaning, it would be wrong (TRD §4-①).
    pub proto_ver: u32,
    pub args: serde_json::Value,
}

/// Reply — **exactly one** per `request_id` (TRD §4-⑤).
///
/// Wire shape = `{"request_id":"…","outcome":{"Ok":…}}` or `{"…","outcome":{"Err":{…}}}`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CommandReply {
    pub request_id: RequestId,
    pub outcome: Result<serde_json::Value, CommandError>,
}

impl CommandReply {
    pub fn ok(request_id: RequestId, value: serde_json::Value) -> Self {
        Self {
            request_id,
            outcome: Ok(value),
        }
    }

    pub fn err(request_id: RequestId, error: CommandError) -> Self {
        Self {
            request_id,
            outcome: Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ErrorCode;

    /// Errors cross the hop **carried inside the reply** — if the wire representation of [`CommandError`] changes, this catches it.
    #[test]
    fn a_reply_round_trips_both_outcomes() {
        let id = RequestId::new();

        let ok = CommandReply::ok(id, serde_json::json!({ "agent_id": "a" }));
        let text = serde_json::to_string(&ok).expect("serialize");
        assert_eq!(
            serde_json::from_str::<CommandReply>(&text).expect("decode"),
            ok
        );

        let err = CommandReply::err(id, CommandError::of(ErrorCode::NotFound, "no agent 'x'"));
        let text = serde_json::to_string(&err).expect("serialize");
        let decoded: CommandReply = serde_json::from_str(&text).expect("decode");
        assert_eq!(decoded, err);
        assert_eq!(
            decoded.outcome.expect_err("error reply").code(),
            ErrorCode::NotFound
        );
    }
}
