//! Wire shape of the connection's first frame (token handshake) — built by the shell and unpacked by the daemon (ADR-0129 0-4 · ADR-0310
//!   decision 9).
//!
//! ★Not a variant of [`crate::AgentCommand`]★: token auth decides whether to keep the socket alive, so the network layer must unpack this
//!   frame without
//! knowing the command vocabulary (ADR-0129 decision 1). Reviving `Auth` in the command enum would leave two diverging definitions.
//!
//! ★★The wire is frozen — the bytes must not move★★: an externally-tagged serde enum with **no** `#[serde(tag=…)]` override, so the frame
//!   is exactly
//! `{"Auth":{"token":"…","protocol_version":N}}` ([`Secret`] is `serde(transparent)`).
//! Senders: the stop path (shell `daemon_client/link/stop.rs`), the shell client (`daemon_client/connection.rs`), the frontend
//!   `wsTransport`, `scripts/engram.mjs`.
//! ★The last two are hand-assembled JS the compiler cannot catch★ (untyped object literals; the `scripts/engram.mjs` path runs without
//!   test coverage —
//! `run-dashboard-release.bat`, `docs/process/S17-llm-control-surface/spec/trd.md`), so the golden string test below keeps the contract.
//! To change the shape, change all senders at once and raise the protocol version.
//! The control CLI `engram` is not a sender — it attaches through the daemon's HTTP control route (`<base>/control/<route>` + Bearer).
//!
//! ★Why a single-variant enum★: a struct would drop the `"Auth"` tag, which is part of the wire contract.

use crate::Secret;

/// The auth frame, valid only as the **first frame** of a connection. The daemon accepts exactly this one within its auth window
/// (`AUTH_TIMEOUT` in the daemon's `link::server`); the layer above rejects the same frame arriving later.
///
/// `protocol_version` is **the version the sender was compiled with** — echoing the value daemon.json gave would make the receiver's comparison always pass and kill the version gate.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AuthFrame {
    Auth {
        token: Secret,
        protocol_version: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★Source of truth for the wire contract★ — a **string literal**, not a comparison with another type (a round-trip against another type passes even when both definitions are wrong together).
    const GOLDEN: &str = r#"{"Auth":{"token":"deadbeefdeadbeef","protocol_version":3}}"#;

    fn golden_frame() -> AuthFrame {
        AuthFrame::Auth {
            token: Secret::new("deadbeefdeadbeef"),
            protocol_version: 3,
        }
    }

    #[test]
    fn serializes_to_the_frozen_wire_bytes() {
        assert_eq!(serde_json::to_string(&golden_frame()).unwrap(), GOLDEN);
    }

    #[test]
    fn deserializes_the_frozen_wire_bytes() {
        let back: AuthFrame = serde_json::from_str(GOLDEN).unwrap();
        assert_eq!(back, golden_frame());
    }

    #[test]
    fn roundtrips() {
        let frame = AuthFrame::Auth {
            token: Secret::new("deadbeef".repeat(8)), // Same 64 chars as production
            protocol_version: 7,
        };
        let json = serde_json::to_string(&frame).unwrap();
        assert_eq!(serde_json::from_str::<AuthFrame>(&json).unwrap(), frame);
    }

    #[test]
    fn debug_does_not_carry_the_token() {
        let token = "deadbeef".repeat(8);
        let frame = AuthFrame::Auth {
            token: Secret::new(token.clone()),
            protocol_version: 7,
        };
        for shown in [format!("{frame:?}"), format!("{frame:#?}")] {
            assert!(!shown.contains(&token), "Debug printed the token: {shown}");
            assert!(
                shown.contains("protocol_version"),
                "only the token is masked: {shown}"
            );
        }
    }

    #[test]
    fn other_tags_are_not_auth() {
        for text in [
            r#"{"ListAgents":{"request_id":"00000000-0000-0000-0000-000000000000"}}"#,
            r#"{"NotACommand":true}"#,
            r#"{}"#,
            r#""Auth""#,
        ] {
            assert!(
                serde_json::from_str::<AuthFrame>(text).is_err(),
                "a frame that is not auth passed: {text}"
            );
        }
    }

    #[test]
    fn syntax_error_and_shape_error_are_distinguishable() {
        // The failure branches of the daemon's `link::auth` (the first frame is not auth / not JSON at all) rely on this classification.
        let shape = serde_json::from_str::<AuthFrame>(r#"{"NotACommand":true}"#).unwrap_err();
        assert!(shape.is_data(), "JSON but a different shape = data error");
        let syntax = serde_json::from_str::<AuthFrame>("not json at all").unwrap_err();
        assert!(!syntax.is_data(), "not JSON = not a data error");
        let empty = serde_json::from_str::<AuthFrame>("").unwrap_err();
        assert!(!empty.is_data(), "empty frame = not a data error (eof)");
    }

    #[test]
    fn unknown_fields_inside_the_object_are_tolerated() {
        // There is no `deny_unknown_fields` — it is additive tolerance where adding a field does not break old servers (removing it
        // silently narrows wire compatibility).
        let with_extra = r#"{"Auth":{"token":"t","protocol_version":3,"future_field":"ignored"}}"#;
        assert_eq!(
            serde_json::from_str::<AuthFrame>(with_extra).unwrap(),
            AuthFrame::Auth {
                token: Secret::new("t"),
                protocol_version: 3,
            }
        );
    }
}
