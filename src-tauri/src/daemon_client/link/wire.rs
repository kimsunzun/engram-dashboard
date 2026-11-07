//! The shell ↔ daemon packet vocabulary as a transport [`Wire`] (TRD S21 trd-3 §3-3 D2 ① · §6-2 U2).
//!
//! - Out = [`AgentCommand`] · In = [`ShellIn`] — JSON text control frames decode to [`AgentEvent`]; binary output
//!   frames pass through undecoded (the output relay keeps its own decode and epoch rules — `frame_relay`).
//! - Pairing = the shell's two pairing functions ([`command_request_id`] · [`event_reply_request_id`]) — one pair, so a
//!   request and its reply can never disagree about which variants pair (3-1 U4).
//! - ★No stream layer★ (ADR-0310 decision 16) — replay→live, the gen fence, epoch filtering and seq continuity stay
//!   with the daemon · `replay_flight` · the frontend. Turning the transport's stream layer on breaks three of them
//!   (TRD §2-5). [`ShellWire`]'s stream key is [`Infallible`], so the crate can neither see a stream fragment nor build
//!   a resume request — a `Subscribe` the gen fence did not issue cannot exist (`replay_flight` header).
// ADR-0310

use std::convert::Infallible;
use std::fmt;

use engram_dashboard_protocol::{AgentCommand, AgentEvent, RequestId};
use engram_dashboard_transport::{DecodeClass, Frame, StreamMark, Wire};

use crate::daemon_client::protocol_state::{command_request_id, event_reply_request_id};

/// What the shell receives from a daemon.
pub enum ShellIn {
    /// A JSON text control frame.
    Event(AgentEvent),
    /// A binary output frame, byte for byte as received — not decoded here.
    Output(Vec<u8>),
}

/// Prints the variant and the byte length only — an output frame is terminal content, and events carry agent
/// payloads (same rule as the transport's `Frame` `Debug` — TRD §5 5).
impl fmt::Debug for ShellIn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShellIn::Event(_) => f.write_str("Event(..)"),
            ShellIn::Output(bytes) => f
                .debug_struct("Output")
                .field("bytes", &bytes.len())
                .finish(),
        }
    }
}

/// Why a text frame did not decode. Only its fixed class word reaches transport events
/// (`TransportEvent::DecodeFailed`) — the parser's message can quote what the peer sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShellDecodeError {
    class: &'static str,
}

impl DecodeClass for ShellDecodeError {
    fn class(&self) -> &'static str {
        self.class
    }
}

/// The shell's [`Wire`] — stateless, one per registry.
pub struct ShellWire;

impl Wire for ShellWire {
    type Out = AgentCommand;
    type In = ShellIn;
    type Tag = RequestId;
    type StreamKey = Infallible;
    type DecodeError = ShellDecodeError;

    fn encode(&self, out: &AgentCommand) -> Frame {
        // Every field of `AgentCommand` serializes infallibly (no non-string map keys; raw slots are already JSON), so
        //   this branch cannot occur.
        Frame::Text(serde_json::to_string(out).expect("AgentCommand always serializes to JSON"))
    }

    fn decode(&self, frame: Frame) -> Result<ShellIn, ShellDecodeError> {
        match frame {
            Frame::Text(text) => serde_json::from_str::<AgentEvent>(&text)
                .map(ShellIn::Event)
                .map_err(|err| {
                    // `?` (Debug) escapes control characters in the quoted peer value.
                    tracing::debug!(error = ?err, "daemon text frame did not decode");
                    ShellDecodeError {
                        class: json_class(&err),
                    }
                }),
            Frame::Binary(bytes) => Ok(ShellIn::Output(bytes)),
            // The transport never passes keepalives to `decode` (`Wire::decode` doc); the arm only makes the match total.
            Frame::Keepalive => Err(ShellDecodeError {
                class: "unexpected keepalive",
            }),
        }
    }

    fn request_tag(&self, out: &AgentCommand) -> Option<RequestId> {
        command_request_id(out)
    }

    fn reply_tag(&self, inn: &ShellIn) -> Option<RequestId> {
        match inn {
            ShellIn::Event(ev) => event_reply_request_id(ev),
            ShellIn::Output(_) => None,
        }
    }

    fn stream_mark(&self, _inn: &ShellIn) -> Option<StreamMark<Infallible>> {
        None
    }

    fn resume_request(&self, key: &Infallible, _after: Option<u64>) -> AgentCommand {
        match *key {}
    }
}

/// A fixed class word for a JSON error — the parser's message can quote the input.
pub(super) fn json_class(err: &serde_json::Error) -> &'static str {
    match err.classify() {
        serde_json::error::Category::Syntax => "json syntax",
        serde_json::error::Category::Data => "json data",
        serde_json::error::Category::Eof => "json eof",
        serde_json::error::Category::Io => "json io",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use engram_dashboard_transport::testing::{
        settle, settle_until, ImmediateHandshake, ManualClock, MemoryNetwork,
    };
    use engram_dashboard_transport::{Address, Incoming, PeerId, PeerState, Policy, Registry};
    use serde_json::value::RawValue;

    fn raw(json: &str) -> Box<RawValue> {
        RawValue::from_string(json.to_owned()).unwrap()
    }

    fn text(ev: &AgentEvent) -> Frame {
        Frame::Text(serde_json::to_string(ev).unwrap())
    }

    #[test]
    fn a_command_with_a_request_id_is_a_request_and_the_rest_are_not() {
        let w = ShellWire;
        let rid = RequestId::new();
        assert_eq!(
            w.request_tag(&AgentCommand::ListAgents { request_id: rid }),
            Some(rid)
        );
        let subscribe = AgentCommand::Subscribe {
            agent_id: uuid::Uuid::new_v4(),
            epoch: None,
            after_seq: None,
        };
        assert_eq!(w.request_tag(&subscribe), None, "Subscribe takes no reply");
        // The shell's answer to a daemon request carries the daemon's key — it must not open a pending slot.
        let outcome = AgentCommand::CommandOutcome {
            reply: raw(&format!(
                r#"{{"request_id":"{}","outcome":{{"Ok":null}}}}"#,
                rid.0
            )),
        };
        assert_eq!(w.request_tag(&outcome), None);
    }

    #[test]
    fn only_a_dedicated_reply_carries_a_reply_tag() {
        let w = ShellWire;
        let rid = RequestId::new();
        let ack = ShellIn::Event(AgentEvent::Ack { request_id: rid });
        assert_eq!(w.reply_tag(&ack), Some(rid));
        let broadcast = ShellIn::Event(AgentEvent::AgentListUpdated { agents: vec![] });
        assert_eq!(w.reply_tag(&broadcast), None);
        // A daemon request carries a key too — but it is the daemon's, not a reply to us.
        let request = ShellIn::Event(AgentEvent::CommandRequest {
            envelope: raw(&format!(
                r#"{{"name":"x","request_id":"{}","owner":"o","proto_ver":1,"args":null}}"#,
                rid.0
            )),
        });
        assert_eq!(w.reply_tag(&request), None);
        assert_eq!(w.reply_tag(&ShellIn::Output(vec![1, 2, 3])), None);
    }

    #[test]
    fn a_text_frame_decodes_to_an_event() {
        let rid = RequestId::new();
        let decoded = ShellWire
            .decode(text(&AgentEvent::Ack { request_id: rid }))
            .unwrap();
        assert!(
            matches!(decoded, ShellIn::Event(AgentEvent::Ack { request_id }) if request_id == rid)
        );
    }

    #[test]
    fn a_binary_frame_passes_through_byte_for_byte() {
        // Not a valid output header on purpose — the wire does not judge binary frames (`frame_relay` does).
        let bytes = vec![0xff, 0x00, 0x7f, 0x10];
        match ShellWire.decode(Frame::Binary(bytes.clone())).unwrap() {
            ShellIn::Output(got) => assert_eq!(got, bytes),
            ShellIn::Event(_) => panic!("binary must not decode to an event"),
        }
    }

    #[test]
    fn an_undecodable_text_frame_reports_a_fixed_class_that_does_not_quote_the_peer() {
        for (input, class) in [
            ("nonsense-from-the-peer", "json syntax"),
            (r#"{"NoSuchVariant":{"secret":"peer-text"}}"#, "json data"),
            (r#"{"Ack":{"#, "json eof"),
        ] {
            let err = ShellWire.decode(Frame::Text(input.into())).unwrap_err();
            assert_eq!(err.class(), class, "input {input:?}");
        }
    }

    #[test]
    fn encode_is_one_text_frame_of_the_serde_json_text() {
        // The daemon reads one JSON text frame per command (`serde_json::to_string` → one text frame).
        let cmd = AgentCommand::ListAgents {
            request_id: RequestId::new(),
        };
        assert_eq!(
            ShellWire.encode(&cmd),
            Frame::Text(serde_json::to_string(&cmd).unwrap())
        );
    }

    #[test]
    fn nothing_is_a_stream_fragment() {
        let w = ShellWire;
        assert!(w.stream_mark(&ShellIn::Output(vec![0; 32])).is_none());
        assert!(w
            .stream_mark(&ShellIn::Event(AgentEvent::AgentListUpdated {
                agents: vec![]
            }))
            .is_none());
    }

    #[tokio::test]
    async fn a_registry_on_the_shell_wire_pairs_a_reply_and_queues_the_rest_in_the_inbox() {
        let net = MemoryNetwork::new();
        let clock = ManualClock::new();
        let registry = Registry::new(
            Arc::new(ShellWire),
            net.dialer(),
            clock.handle(),
            Policy::default(),
        );
        let (peer, mut inbox) = registry.add(
            PeerId::new("daemon"),
            Address::new("mem"),
            Arc::new(ImmediateHandshake),
        );
        settle_until("live", || peer.peer_state() == PeerState::Live).await;
        let endpoint = net.accept().expect("a link was opened");

        let rid = RequestId::new();
        let cmd = AgentCommand::ListAgents { request_id: rid };
        let sent = serde_json::to_string(&cmd).unwrap();
        let task = {
            let peer = peer.clone();
            tokio::spawn(async move { peer.request(cmd).await })
        };
        settle().await;
        assert_eq!(endpoint.frames(), vec![Frame::Text(sent)]);

        endpoint.push(text(&AgentEvent::AgentListUpdated { agents: vec![] }));
        endpoint.push(Frame::Binary(vec![9, 8, 7]));
        endpoint.push(text(&AgentEvent::Ack { request_id: rid }));
        // Not a bare `task.await` — under the manual clock the request timeout never fires, so a reply that fails to
        //   pair would hang the suite instead of failing it.
        settle_until("the reply wakes the request", || task.is_finished()).await;

        let reply = task.await.unwrap().expect("the reply wakes the request");
        assert!(
            matches!(reply, ShellIn::Event(AgentEvent::Ack { request_id }) if request_id == rid)
        );

        let mut messages = Vec::new();
        while let Some(item) = inbox.try_recv() {
            if let Incoming::Message(arrival) = item {
                messages.push(arrival.msg);
            }
        }
        assert_eq!(
            messages.len(),
            2,
            "the broadcast and the output frame, not the reply: {messages:?}"
        );
        assert!(matches!(
            messages[0],
            ShellIn::Event(AgentEvent::AgentListUpdated { .. })
        ));
        assert!(matches!(&messages[1], ShellIn::Output(bytes) if bytes == &vec![9, 8, 7]));
    }
}
