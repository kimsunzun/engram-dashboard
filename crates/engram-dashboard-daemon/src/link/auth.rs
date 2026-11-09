//! The first-frame auth decision — token (constant time) and protocol version, the same rules the net layer applied.
//!
//! Both expected values are **injected** by the assembly (`run_accept_loop`): this module decides only "is the token the
//! client stated equal to the one I was given" and "is the version the client stated equal to the one I was given" — it
//! reads neither daemon.json nor the protocol version constant.
//!
//! ★The refusal text never quotes the frame★ — the frame can carry the token, so logs carry only serde's error class
//! and position (`docs/reference/logging-conventions.md`).
// ADR-0310

use engram_dashboard_protocol::handshake::AuthFrame;
use engram_dashboard_transport::RejectKind;

/// The values one daemon accepts on the first frame. `Debug` is not derived — it holds the token.
pub(crate) struct AuthCheck {
    token: String,
    protocol_version: u32,
}

/// Why the first frame was refused. `message` goes to the client as the handshake `Error` text and as the close
/// reason; `kind` picks the close code (`RejectKind::close_code`).
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
                    tracing::warn!(%peer, "auth 실패: 토큰 불일치 — close");
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
                        "auth 실패: protocol_version 불일치 — close"
                    );
                    return Err(Refusal::terminal(format!(
                        "protocol_version mismatch: client {protocol_version} != server {}",
                        self.protocol_version
                    )));
                }
                tracing::info!(%peer, "auth 성공");
                Ok(())
            }
            // ★The boundary of the two failure messages★: this module does not know upper-layer commands, so it cannot decide "parses as an
            //   upper-layer command but is not auth" — instead it uses the error classification serde already computed (zero re-parsing):
            //     · data error  = JSON is valid but not this shape → "expected auth"
            //     · anything else (syntax/eof/io) = not JSON at all → "the frame itself is wrong"
            //   It is diagnostic text for humans; no sender branches on the wording.
            // ★Do not carry the error as `{e}`★: the Display of `{"Auth":"<토큰>"}` emits that string as is, and the token leaks into
            //   the log. Leave only the classification and position — the regression defense is
            //   `auth_parse_failure_log_fields_do_not_carry_the_payload` below.
            Err(e) if e.is_data() => {
                tracing::warn!(
                    %peer,
                    kind = ?e.classify(),
                    line = e.line(),
                    column = e.column(),
                    "첫 frame 이 Auth 가 아님 — close"
                );
                Err(Refusal::terminal("expected Auth as first frame"))
            }
            // For the same reason as the branch above, do not carry Display (the token can arrive on this branch too).
            Err(e) => {
                tracing::warn!(
                    %peer,
                    kind = ?e.classify(),
                    line = e.line(),
                    column = e.column(),
                    "첫 frame 파싱 실패 — close"
                );
                Err(Refusal::terminal("invalid first frame"))
            }
        }
    }

    /// A binary first frame — auth is a text frame.
    pub(crate) fn refuse_binary(&self, peer: &str) -> Refusal {
        tracing::warn!(%peer, "첫 frame 이 Text 가 아님 — close");
        Refusal::terminal("expected Auth text frame")
    }
}

/// Exposing the length is harmless since the token length is fixed (64 hex chars).
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

    /// An arbitrary protocol version. ★Deliberately not the production version★ — a check that read the constant back
    /// instead of the injected value would diverge from it.
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
        assert!(constant_time_eq(&a, &"a".repeat(64)), "동일 토큰은 true");
        assert!(!constant_time_eq(&a, &"b".repeat(64)), "다른 토큰은 false");
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
            "Display 가 토큰을 싣지 않으면 이 테스트가 지키는 게 없다: {e}"
        );
        let logged = format!("{:?} {} {}", e.classify(), e.line(), e.column());
        assert!(
            !logged.contains(&token),
            "로깅 필드에 토큰이 실렸다: {logged}"
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
            "두 숫자 모두 주입값 기준이어야(상수 하드코딩이면 어긋난다)"
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
