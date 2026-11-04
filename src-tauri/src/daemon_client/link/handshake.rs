//! The shell side of the daemon handshake as a transport [`Handshake`] (TRD S21 trd-3 §6-2 U2 · §3-0 ⑤ · D20 ①).
//!
//! Two steps: `start` sends the Auth frame, the daemon's `Hello` ends the handshake. ★The kind of a refusal is never
//! read from the daemon's text★ — an `Error` frame is logged by length only and the handshake waits for the close
//! code, which the transport turns into "terminal" (4001 — no budget) or "retry" (any other refusal code — budget).
//! Anything else that arrives before `Hello` is dropped, the same as the old driver's `wait_for_hello`.
//!
//! The credential comes from a [`CredentialSlot`] that the dialer of the same registration fills on every dial
//! (`link::dialer`) — the dial and the handshake of one attempt run in that order on one supervisor task, so a
//! credential re-read after a retryable refusal reaches the next Auth frame.
// ADR-0310

use std::sync::{Arc, Mutex, PoisonError};

use engram_dashboard_protocol::handshake::AuthFrame;
use engram_dashboard_protocol::{AgentEvent, Secret, PROTOCOL_VERSION};
use engram_dashboard_transport::{Frame, Handshake, HandshakeStep, RejectKind};

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
}

impl Handshake for ShellHandshake {
    fn start(&self) -> HandshakeStep {
        // The dialer only hands out a link after it wrote a credential (a live endpoint, or the last-known one that
        //   came with one), so an empty slot means a dialer that is not this handshake's pair.
        let Some(token) = self.slot.get() else {
            return HandshakeStep::Reject {
                kind: RejectKind::Retryable,
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
                tracing::warn!(
                    // Length only — the text is the daemon's and may echo what we sent, the token included.
                    detail_bytes = message.len(),
                    "daemon refused the handshake — waiting for its close code"
                );
                HandshakeStep::Await
            }
            Ok(_) | Err(_) => HandshakeStep::Await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn an_error_frame_waits_for_the_close_code_instead_of_rejecting() {
        let error = text(&AgentEvent::Error {
            request_id: None,
            message: "auth failed".into(),
        });
        assert!(matches!(
            handshake_with(TOKEN).on_frame(&error),
            HandshakeStep::Await
        ));
    }

    #[test]
    fn an_error_frame_that_echoes_the_token_is_logged_without_its_text() {
        let lines = Arc::new(Mutex::new(Vec::new()));
        let echo = text(&AgentEvent::Error {
            request_id: None,
            message: format!("bad token {TOKEN}"),
        });
        let step = tracing::subscriber::with_default(Lines(lines.clone()), || {
            handshake_with(TOKEN).on_frame(&echo)
        });
        let lines = lines.lock().unwrap();
        assert_eq!(lines.len(), 1, "the refusal is logged: {lines:?}");
        assert!(!lines[0].contains(TOKEN), "{lines:?}");
        assert!(!format!("{step:?}").contains(TOKEN));
    }

    /// Every field of every event on this thread, as `name=value`.
    struct Lines(Arc<Mutex<Vec<String>>>);

    struct Fields<'a>(&'a mut String);

    impl tracing::field::Visit for Fields<'_> {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.0.push_str(&format!("{}={value:?} ", field.name()));
        }
    }

    impl tracing::Subscriber for Lines {
        fn register_callsite(
            &self,
            _: &'static tracing::Metadata<'static>,
        ) -> tracing::subscriber::Interest {
            tracing::subscriber::Interest::sometimes()
        }
        fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            let mut line = String::new();
            event.record(&mut Fields(&mut line));
            self.0.lock().unwrap().push(line);
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
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
