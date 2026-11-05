//! The shell side of the daemon handshake as a transport [`Handshake`] (TRD S21 trd-3 §6-2 U2 · §3-0 ⑤ · D20 ①).
//!
//! Two steps: `start` sends the Auth frame, the daemon's `Hello` ends the handshake. ★An `Error` frame before `Hello`
//! ends the handshake as a terminal refusal★ (`HANDSHAKE_REJECTED` — no retry, no budget): the daemon closes without
//! a close code after it (net `ws.rs` `send_error_and_close`), which the transport would read as "could not connect"
//! and retry without limit, and no refusal is retried on the local path (ADR-0346). Which refusal it was (token ·
//! version · anything else) is never read from the daemon's text — the text is only logged, masked. Anything else that
//! arrives before `Hello` is dropped, the same as the old driver's `wait_for_hello`.
//!
//! The credential comes from a [`CredentialSlot`] that the dialer of the same registration fills on every dial
//! (`link::dialer`) — the dial and the handshake of one attempt run in that order on one supervisor task, so the
//! credential read for an attempt reaches its Auth frame.
// ADR-0310
// ADR-0346

use std::sync::{Arc, Mutex, PoisonError};

use engram_dashboard_protocol::handshake::AuthFrame;
use engram_dashboard_protocol::{AgentEvent, Secret, PROTOCOL_VERSION};
use engram_dashboard_transport::{mask_reason, Frame, Handshake, HandshakeStep, RejectKind};

/// The credential the next handshake of one registration sends — shared by that registration's dial (writer) and
/// handshake (reader). Empty until the first dial resolved a live endpoint. `Debug` shows no value ([`Secret`]).
#[derive(Debug, Default)]
pub struct CredentialSlot(Mutex<Option<Secret>>);

impl CredentialSlot {
    pub(super) fn set(&self, credential: Secret) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(credential);
    }

    fn get(&self) -> Option<Secret> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// See the module doc. Built by `LocalDaemonDialer::register` together with the slot it reads.
#[derive(Debug)]
pub struct ShellHandshake {
    slot: Arc<CredentialSlot>,
}

impl ShellHandshake {
    pub(super) fn new(slot: Arc<CredentialSlot>) -> Self {
        Self { slot }
    }

    /// The daemon's text as it may be logged — masked (`mask_reason`), or `None` when it carries the token we sent, as
    /// it is or once the hidden characters masking drops are gone (they can split a token that masking then joins).
    /// ★Both checks run before masking truncates★ — a token past the cut would survive in part.
    fn loggable(&self, message: &str) -> Option<String> {
        if let Some(token) = self.slot.get().filter(|t| !t.expose().is_empty()) {
            let token = token.expose();
            if message.contains(token) || without_hidden(message).contains(token) {
                return None;
            }
        }
        Some(mask_reason(message))
    }
}

/// `text` without the characters [`mask_reason`] drops. ★`mask_reason` stays the one source of what is hidden★ (the
/// transport does not export its predicate): a non-ASCII character is asked by masking it alone — it masks to nothing
/// when hidden, and one character is never truncated. ASCII takes a fast path that allocates nothing: the only ASCII
/// characters masking drops are the controls (`char::is_control`; its other hidden ranges are all non-ASCII).
fn without_hidden(text: &str) -> String {
    let mut buf = [0u8; 4];
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        let hidden = if c.is_ascii() {
            c.is_ascii_control()
        } else {
            mask_reason(c.encode_utf8(&mut buf)).is_empty()
        };
        if !hidden {
            out.push(c);
        }
    }
    out
}

impl Handshake for ShellHandshake {
    fn start(&self) -> HandshakeStep {
        // The dialer only hands out a link after it wrote a credential (a live endpoint, or the last-known one that
        //   came with one), so an empty slot means a dialer that is not this handshake's pair. Terminal: no refusal is
        //   retried on the local path (ADR-0346).
        let Some(token) = self.slot.get() else {
            return HandshakeStep::Reject {
                kind: RejectKind::Terminal,
                last: None,
                reason: "no credential for this daemon".into(),
            };
        };
        // Fix C: our compiled version, never the one daemon.json reported — echoing it would pass every
        //   version check on the daemon side.
        let auth = AuthFrame::Auth {
            token,
            protocol_version: PROTOCOL_VERSION,
        };
        // Serializing a two-field struct of a string and an integer cannot fail.
        let text = serde_json::to_string(&auth).expect("AuthFrame serializes");
        HandshakeStep::Send(Frame::Text(text))
    }

    fn on_frame(&self, frame: &Frame) -> HandshakeStep {
        let Frame::Text(text) = frame else {
            return HandshakeStep::Await;
        };
        match serde_json::from_str::<AgentEvent>(text) {
            Ok(AgentEvent::Hello { .. }) => HandshakeStep::Done,
            Ok(AgentEvent::Error { message, .. }) => {
                match self.loggable(&message) {
                    Some(detail) => tracing::warn!(
                        %detail,
                        "the daemon refused the handshake — not retrying"
                    ),
                    None => tracing::warn!(
                        detail_bytes = message.len(),
                        "the daemon refused the handshake — not retrying (its text carries the token and is not logged)"
                    ),
                }
                HandshakeStep::Reject {
                    kind: RejectKind::Terminal,
                    last: None,
                    reason: "the shell gives up after the daemon's refusal".into(),
                }
            }
            Ok(_) | Err(_) => HandshakeStep::Await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::super::test_log::capture;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn handshake_with(token: &str) -> ShellHandshake {
        let slot = Arc::new(CredentialSlot::default());
        slot.set(Secret::new(token));
        ShellHandshake::new(slot)
    }

    fn text(ev: &AgentEvent) -> Frame {
        Frame::Text(serde_json::to_string(ev).unwrap())
    }

    fn hello() -> Frame {
        text(&AgentEvent::Hello {
            protocol_version: PROTOCOL_VERSION,
            daemon_version: "0.0.0".into(),
            capabilities: None,
        })
    }

    #[test]
    fn start_sends_the_auth_frame_with_the_slot_token_and_the_compiled_protocol_version() {
        let golden =
            format!(r#"{{"Auth":{{"token":"{TOKEN}","protocol_version":{PROTOCOL_VERSION}}}}}"#);
        match handshake_with(TOKEN).start() {
            HandshakeStep::Send(Frame::Text(sent)) => assert_eq!(sent, golden),
            other => panic!("expected the Auth text frame, got {other:?}"),
        }
    }

    #[test]
    fn start_without_a_credential_is_a_terminal_refusal() {
        let handshake = ShellHandshake::new(Arc::new(CredentialSlot::default()));
        assert!(matches!(
            handshake.start(),
            HandshakeStep::Reject {
                kind: RejectKind::Terminal,
                ..
            }
        ));
    }

    #[test]
    fn start_reads_the_slot_at_call_time() {
        let slot = Arc::new(CredentialSlot::default());
        slot.set(Secret::new("first"));
        let handshake = ShellHandshake::new(slot.clone());
        slot.set(Secret::new("second"));
        match handshake.start() {
            HandshakeStep::Send(Frame::Text(sent)) => {
                assert!(sent.contains(r#""token":"second""#), "{sent}")
            }
            other => panic!("expected the Auth text frame, got {other:?}"),
        }
    }

    #[test]
    fn hello_ends_the_handshake() {
        assert!(matches!(
            handshake_with(TOKEN).on_frame(&hello()),
            HandshakeStep::Done
        ));
    }

    #[test]
    fn an_error_frame_is_a_terminal_refusal_whose_reason_quotes_nothing_the_daemon_sent() {
        let error = text(&AgentEvent::Error {
            request_id: None,
            message: "auth failed".into(),
        });
        match handshake_with(TOKEN).on_frame(&error) {
            HandshakeStep::Reject {
                kind: RejectKind::Terminal,
                last: None,
                reason,
            } => assert!(!reason.contains("auth failed"), "{reason}"),
            other => panic!("expected a terminal refusal, got {other:?}"),
        }
    }

    #[test]
    fn an_error_frame_is_logged_masked() {
        let lines = Arc::new(Mutex::new(Vec::new()));
        let error = text(&AgentEvent::Error {
            request_id: None,
            message: "protocol_version mismatch\r\nforged line\u{202e}".into(),
        });
        capture(lines.clone(), || handshake_with(TOKEN).on_frame(&error));
        let lines = lines.lock().unwrap();
        assert_eq!(lines.len(), 1, "the refusal is logged: {lines:?}");
        assert!(
            lines[0].contains("protocol_version mismatchforged line"),
            "{lines:?}"
        );
        assert!(
            !lines[0].contains('\n') && !lines[0].contains('\u{202e}'),
            "{lines:?}"
        );
    }

    #[test]
    fn an_error_frame_that_echoes_the_token_is_logged_without_its_text() {
        let lines = Arc::new(Mutex::new(Vec::new()));
        let echo = text(&AgentEvent::Error {
            request_id: None,
            message: format!("bad token {TOKEN}"),
        });
        let step = capture(lines.clone(), || handshake_with(TOKEN).on_frame(&echo));
        let lines = lines.lock().unwrap();
        assert_eq!(lines.len(), 1, "the refusal is logged: {lines:?}");
        assert!(!lines[0].contains(TOKEN), "{lines:?}");
        assert!(!format!("{step:?}").contains(TOKEN));
    }

    #[test]
    fn a_token_split_by_hidden_characters_past_the_masking_cut_is_not_logged() {
        let lines = Arc::new(Mutex::new(Vec::new()));
        let (head, tail) = TOKEN.split_at(32);
        // Past the 256-byte cut once the hidden characters are gone, so masking keeps only part of the token.
        let echo = text(&AgentEvent::Error {
            request_id: None,
            message: format!("{}{head}\u{200b}{tail}", "x".repeat(230)),
        });
        capture(lines.clone(), || handshake_with(TOKEN).on_frame(&echo));
        let lines = lines.lock().unwrap();
        assert_eq!(lines.len(), 1, "the refusal is logged: {lines:?}");
        assert!(lines[0].contains("detail_bytes="), "{lines:?}");
        assert!(!lines[0].contains(&TOKEN[..16]), "{lines:?}");
    }

    #[test]
    fn without_hidden_drops_exactly_what_masking_drops() {
        let text = "a\u{1b}[31mb\r\nc\td\u{7f}e\u{9b}f ok\u{202e}g\u{200b}h\u{e0041}i 버전";
        assert_eq!(without_hidden(text), mask_reason(text));
        for byte in 0u8..0x80 {
            let c = char::from(byte);
            assert_eq!(
                without_hidden(&c.to_string()).is_empty(),
                mask_reason(&c.to_string()).is_empty(),
                "{byte:#x}"
            );
        }
    }

    #[test]
    fn any_other_frame_before_hello_is_dropped() {
        let handshake = handshake_with(TOKEN);
        for frame in [
            text(&AgentEvent::AgentListUpdated { agents: vec![] }),
            Frame::Binary(vec![1, 2, 3]),
            Frame::Text("not json".into()),
        ] {
            assert!(
                matches!(handshake.on_frame(&frame), HandshakeStep::Await),
                "{frame:?}"
            );
        }
    }

    #[test]
    fn debug_of_the_handshake_and_its_slot_shows_no_token() {
        let handshake = handshake_with(TOKEN);
        let printed = format!("{handshake:?} {:?}", handshake.slot);
        assert!(!printed.contains(TOKEN), "{printed}");
    }
}
