//! The first-frame auth decision — token (constant time) and protocol version.
//!
//! Both expected values are injected by the assembly (`run_accept_loop`): this module only decides whether the stated token and version equal the ones it was given — it reads neither daemon.json nor the protocol version constant.
//!
//! ★The refusal text never quotes the frame★ — the frame can carry the token, so logs carry only serde's error class and position (`docs/reference/logging-conventions.md`).
// ADR-0310

use engram_dashboard_protocol::handshake::AuthFrame;
use engram_dashboard_transport::RejectKind;

/// The values one daemon accepts on the first frame. `Debug` is not derived — it holds the token.
pub(crate) struct AuthCheck {
    token: String,
    protocol_version: u32,
}

/// Why the first frame was refused. `message` goes to the client as the handshake `Error` text and the close reason; `kind` picks the close code (`RejectKind::close_code`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Refusal {
    pub(crate) kind: RejectKind,
    pub(crate) message: String,
}

impl Refusal {
    fn terminal(message: impl Into<String>) -> Self {
        Self {
            kind: RejectKind::Terminal,
            message: message.into(),
        }
    }
}

impl AuthCheck {
    pub(crate) fn new(token: String, protocol_version: u32) -> Self {
        Self {
            token,
            protocol_version,
        }
    }

    /// Decides one first text frame. `peer` is the remote address, for the log lines only.
    pub(crate) fn check_text(&self, peer: &str, text: &str) -> Result<(), Refusal> {
        match serde_json::from_str::<AuthFrame>(text) {
            Ok(AuthFrame::Auth {
                token,
                protocol_version,
            }) => {
                // Security: no logging of the token value.
                if !constant_time_eq(token.expose(), &self.token) {
                    tracing::warn!(%peer, "auth failed: token mismatch — close");
                    // A stale credential may pass on the next dial (TRD trd-3 §3-0 ⑤ D20 ①) — only the close code says so.
                    return Err(Refusal {
                        kind: RejectKind::Retryable,
                        message: "auth failed".into(),
                    });
                }
                if protocol_version != self.protocol_version {
                    tracing::warn!(
                        %peer,
                        client = protocol_version,
                        server = self.protocol_version,
                        "auth failed: protocol_version mismatch — close"
                    );
                    return Err(Refusal::terminal(format!(
                        "protocol_version mismatch: client {protocol_version} != server {}",
                        self.protocol_version
                    )));
                }
                tracing::info!(%peer, "auth succeeded");
                Ok(())
            }
            // ★Boundary of the two failure messages★: this module does not know upper-layer commands, so it cannot decide "parses as a command but is not auth" — it uses the error classification serde already computed:
            //     · data error = valid JSON, wrong shape → "expected auth"
            //     · anything else (syntax/eof/io) = not JSON → "the frame itself is wrong"
            //   Diagnostic text for humans; no sender branches on the wording.
            // ★Do not carry the error as `{e}`★: Display of `{"Auth":"<token>"}` emits that string and leaks the token into the log. Leave only classification and position — regression defense: `auth_parse_failure_log_fields_do_not_carry_the_payload` below.
            Err(e) if e.is_data() => {
                tracing::warn!(
                    %peer,
                    kind = ?e.classify(),
                    line = e.line(),
                    column = e.column(),
                    "first frame is not Auth — close"
                );
                Err(Refusal::terminal("expected Auth as first frame"))
            }
            // Same reason — do not carry Display (the token can arrive on this branch too).
            Err(e) => {
                tracing::warn!(
                    %peer,
                    kind = ?e.classify(),
                    line = e.line(),
                    column = e.column(),
                    "first frame parse failed — close"
                );
                Err(Refusal::terminal("invalid first frame"))
            }
        }
    }

    /// A binary first frame — auth is a text frame.
    pub(crate) fn refuse_binary(&self, peer: &str) -> Refusal {
        tracing::warn!(%peer, "first frame is not Text — close");
        Refusal::terminal("expected Auth text frame")
    }
}

/// Exposing the length is harmless: the token length is fixed (64 hex chars).
fn constant_time_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use engram_dashboard_protocol::Secret;

    /// An arbitrary protocol version. ★Deliberately not the production version★ — a check reading the constant back instead of the injected value would diverge from it.
    const TEST_WIRE_VERSION: u32 = 4242;

    fn auth_text(token: &str, protocol_version: u32) -> String {
        serde_json::to_string(&AuthFrame::Auth {
            token: Secret::new(token),
            protocol_version,
        })
        .unwrap()
    }

    #[test]
    fn constant_time_eq_correctness() {
        let a = "a".repeat(64);
        assert!(
            constant_time_eq(&a, &"a".repeat(64)),
            "identical tokens give true"
        );
        assert!(
            !constant_time_eq(&a, &"b".repeat(64)),
            "different tokens give false"
        );
        assert!(!constant_time_eq(&a, &"a".repeat(63)));
        assert!(!constant_time_eq(&a, &"a".repeat(65)));
        let mut almost = "a".repeat(64);
        almost.replace_range(63..64, "b");
        assert!(!constant_time_eq(&a, &almost));
        assert!(constant_time_eq("", ""));
    }

    #[test]
    fn auth_parse_failure_log_fields_do_not_carry_the_payload() {
        let token = "deadbeef".repeat(8); // Same 64 chars as production
        let e = serde_json::from_str::<AuthFrame>(&format!(r#"{{"Auth":"{token}"}}"#)).unwrap_err();
        assert!(
            e.to_string().contains(&token),
            "if Display does not carry the token, this test guards nothing: {e}"
        );
        let logged = format!("{:?} {} {}", e.classify(), e.line(), e.column());
        assert!(
            !logged.contains(&token),
            "the token got into a logging field: {logged}"
        );
    }

    #[test]
    fn the_right_token_and_version_pass() {
        let check = AuthCheck::new("tok".into(), TEST_WIRE_VERSION);
        assert_eq!(
            check.check_text("peer", &auth_text("tok", TEST_WIRE_VERSION)),
            Ok(())
        );
    }

    #[test]
    fn each_refusal_carries_the_net_layer_text() {
        let check = AuthCheck::new("tok".into(), TEST_WIRE_VERSION);
        let cases = [
            (auth_text("other", TEST_WIRE_VERSION), "auth failed"),
            (
                r#"{"ListAgents":{}}"#.to_string(),
                "expected Auth as first frame",
            ),
            ("not json".to_string(), "invalid first frame"),
        ];
        for (text, message) in cases {
            let refusal = check.check_text("peer", &text).unwrap_err();
            assert_eq!(refusal.message, message, "{text}");
        }
        assert_eq!(
            check.refuse_binary("peer").message,
            "expected Auth text frame"
        );
    }

    #[test]
    fn a_version_mismatch_names_both_injected_numbers() {
        let check = AuthCheck::new("tok".into(), TEST_WIRE_VERSION);
        let refusal = check
            .check_text("peer", &auth_text("tok", TEST_WIRE_VERSION + 1))
            .unwrap_err();
        assert_eq!(
            refusal.message,
            format!(
                "protocol_version mismatch: client {} != server {}",
                TEST_WIRE_VERSION + 1,
                TEST_WIRE_VERSION
            ),
            "both numbers must be based on the injected value (a hardcoded constant would diverge)"
        );
        assert_eq!(refusal.kind, RejectKind::Terminal);
    }

    #[test]
    fn only_a_token_mismatch_is_retryable() {
        let check = AuthCheck::new("tok".into(), TEST_WIRE_VERSION);
        assert_eq!(
            check
                .check_text("peer", &auth_text("other", TEST_WIRE_VERSION))
                .unwrap_err()
                .kind,
            RejectKind::Retryable
        );
        for text in [r#"{"ListAgents":{}}"#, "not json"] {
            assert_eq!(
                check.check_text("peer", text).unwrap_err().kind,
                RejectKind::Terminal,
                "{text}"
            );
        }
        assert_eq!(check.refuse_binary("peer").kind, RejectKind::Terminal);
    }
}
