//! The daemon side of the shell handshake as a transport [`Handshake`] — one per accepted link.
//!
//! The daemon waits for the client's first frame and decides it with [`AuthCheck`]: pass = the link goes live, refusal =
//! the `Error` text frame the upper layer encodes (`ConnectionHandlerFactory::handshake_error_frame`) then a close. The
//! auth deadline is the transport's (`Policy::connect_timeout`, measured from the end of the upgrade). `Hello` is not sent
//! here — after a `Send` the transport always awaits a reply, so the upper layer puts `Hello` in from `on_connect`.
// ADR-0310

use std::sync::Arc;

use engram_dashboard_transport::{Frame, Handshake, HandshakeStep};

use super::auth::{AuthCheck, Refusal};
use crate::conn_port::ConnectionHandlerFactory;

pub(crate) struct DaemonHandshake {
    auth: Arc<AuthCheck>,
    handlers: Arc<dyn ConnectionHandlerFactory>,
    /// The remote address, for the log lines only.
    peer: String,
}

impl DaemonHandshake {
    pub(crate) fn new(
        auth: Arc<AuthCheck>,
        handlers: Arc<dyn ConnectionHandlerFactory>,
        peer: String,
    ) -> Self {
        Self {
            auth,
            handlers,
            peer,
        }
    }

    fn reject(&self, refusal: Refusal) -> HandshakeStep {
        HandshakeStep::Reject {
            kind: refusal.kind,
            // `None` = the upper layer could not encode the text — close with no body (as the net layer did).
            last: self
                .handlers
                .handshake_error_frame(&refusal.message)
                .map(Frame::Text),
            reason: refusal.message,
        }
    }
}

impl Handshake for DaemonHandshake {
    fn start(&self) -> HandshakeStep {
        HandshakeStep::Await
    }

    fn on_frame(&self, frame: &Frame) -> HandshakeStep {
        match frame {
            Frame::Text(text) => match self.auth.check_text(&self.peer, text) {
                Ok(()) => HandshakeStep::Done,
                Err(refusal) => self.reject(refusal),
            },
            Frame::Binary(_) => self.reject(self.auth.refuse_binary(&self.peer)),
            // The transport answers keepalives itself and never hands one here.
            Frame::Keepalive => HandshakeStep::Await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use engram_dashboard_protocol::handshake::AuthFrame;
    use engram_dashboard_protocol::Secret;
    use engram_dashboard_transport::RejectKind;

    use crate::conn_port::{ConnId, ConnectionHandler};

    struct EchoFactory;

    impl ConnectionHandlerFactory for EchoFactory {
        fn handler_for(&self, _conn_id: ConnId) -> Arc<dyn ConnectionHandler> {
            unreachable!("the handshake never makes a handler")
        }
        fn handshake_error_frame(&self, message: &str) -> Option<String> {
            Some(format!("error:{message}"))
        }
    }

    fn handshake() -> DaemonHandshake {
        DaemonHandshake::new(
            Arc::new(AuthCheck::new("tok".into(), 7)),
            Arc::new(EchoFactory),
            "peer".into(),
        )
    }

    fn auth(token: &str) -> Frame {
        Frame::Text(
            serde_json::to_string(&AuthFrame::Auth {
                token: Secret::new(token),
                protocol_version: 7,
            })
            .unwrap(),
        )
    }

    #[test]
    fn the_daemon_waits_for_the_client_first() {
        assert!(matches!(handshake().start(), HandshakeStep::Await));
    }

    #[test]
    fn a_good_auth_frame_ends_the_handshake() {
        assert!(matches!(
            handshake().on_frame(&auth("tok")),
            HandshakeStep::Done
        ));
    }

    #[test]
    fn a_refusal_sends_the_upper_layer_error_frame_then_closes() {
        match handshake().on_frame(&auth("other")) {
            HandshakeStep::Reject { kind, last, reason } => {
                assert_eq!(kind, RejectKind::Retryable);
                assert_eq!(last, Some(Frame::Text("error:auth failed".into())));
                assert_eq!(reason, "auth failed");
            }
            other => panic!("{other:?}"),
        }
        match handshake().on_frame(&Frame::Binary(vec![1])) {
            HandshakeStep::Reject { kind, last, .. } => {
                assert_eq!(kind, RejectKind::Terminal);
                assert_eq!(
                    last,
                    Some(Frame::Text("error:expected Auth text frame".into()))
                );
            }
            other => panic!("{other:?}"),
        }
    }
}
