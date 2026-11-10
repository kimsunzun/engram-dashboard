//! Wire shape of the connection's first frame (token handshake) — built by the shell and unpacked by the daemon (ADR-0129 0-4 · ADR-0310 decision 9).
//!
//! ★Not a variant of the command enum ([`crate::AgentCommand`])★: token auth decides 「이 소켓을 살릴지」 (whether to keep this socket alive), so even if the
//! network layer that makes that decision does not know the command vocabulary, it must be able to unpack this one frame (ADR-0129 decision 1).
//! So it is a separate type, and reviving `Auth` in the command enum would make two definitions coexist and diverge (that enum's comment).
//!
//! ★★The wire is frozen — the bytes must not move★★: it is an externally-tagged serde enum with **no** `#[serde(tag=…)]`-style
//! override. So the frame is exactly
//!   `{"Auth":{"token":"…","protocol_version":N}}`
//! . The token field's [`Secret`] is `serde(transparent)`, so the bytes are unchanged. The senders that build this shape —
//! the stop path (shell `daemon_client/link/stop.rs`) · the daemon client shell (`daemon_client/connection.rs`) · the frontend
//! `wsTransport` · `scripts/engram.mjs`.
//! ★The last two are **hand-assembled JS, which the compiler cannot catch**★: both build the frame as an untyped object literal, and
//! the `scripts/engram.mjs` side is a path that runs even though no test covers it (`run-dashboard-release.bat` ·
//! `docs/process/S17-llm-control-surface/spec/trd.md`). So what keeps the contract is the golden string test below.
//! To change the shape, change those senders **at the same time** and raise the protocol version.
//! ★The control CLI `engram` is not on that list★ — it attaches through the daemon's HTTP control route (`<base>/control/<route>` + Bearer) and
//! does not build this frame, so even if the wire changes it is not affected.
//!
//! ★Why a single-variant enum★: a struct would become `{"token":…}` and the tag would disappear. The tag `"Auth"` is
//! part of the wire contract, so it must be an enum (the price is the redundancy `AuthFrame::Auth`, but the redundancy
//! makes the wire tag visible).

use crate::Secret;

/// The auth frame, valid only as the **first frame** of a connection. The daemon accepts exactly this one within
/// its auth window (`AUTH_TIMEOUT` in the daemon's `link::server`); the layer above rejects the same frame arriving later.
///
/// `token` is daemon.json's 256-bit hex · `protocol_version` is **the version the sending side was compiled with** —
/// if the value daemon.json gave were echoed back, the receiver's version comparison would always pass and the version gate would die.
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

    /// ★Source of truth for the wire contract★ — a **string literal**, not a comparison with another type. If changed to a form that
    ///   matches a round-trip against another type, it **passes even when both definitions are wrong together**.
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
            assert!(!shown.contains(&token), "Debug 가 토큰을 찍었다: {shown}");
            assert!(
                shown.contains("protocol_version"),
                "가린 것은 토큰뿐: {shown}"
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
                "auth 가 아닌 프레임이 통과했다: {text}"
            );
        }
    }

    #[test]
    fn syntax_error_and_shape_error_are_distinguishable() {
        // The failure branches of the daemon's `link::auth` (the first frame is not auth / not JSON at all) rely on this classification.
        // It is a value serde_json has already computed, so the branches split without re-parsing.
        let shape = serde_json::from_str::<AuthFrame>(r#"{"NotACommand":true}"#).unwrap_err();
        assert!(shape.is_data(), "JSON 이지만 모양이 다름 = data 에러");
        let syntax = serde_json::from_str::<AuthFrame>("not json at all").unwrap_err();
        assert!(!syntax.is_data(), "JSON 이 아님 = data 에러가 아님");
        let empty = serde_json::from_str::<AuthFrame>("").unwrap_err();
        assert!(!empty.is_data(), "빈 프레임 = data 에러가 아님(eof)");
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
