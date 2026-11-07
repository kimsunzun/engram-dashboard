//! The whole packet vocabulary the consumer supplies.
//!
//! ★This trait is the reason this crate exists★ — delete it and `Out`/`In`/`Tag` disappear and the crate
//! becomes a byte pipe (candidate C, rejected by ADR-0177). That is why this one place is **generic**, not dyn.

use std::hash::Hash;

use crate::frame::Frame;
use crate::stream::StreamMark;

/// The consumer's packet vocabulary. ★The crate knows no name inside it★ — it only looks at whether two are the same (`Eq`) and whether one is present (`Option`).
///
/// ★The roles (request/response/notification/stream) are not a new vocabulary but combinations of the accessors below★. The outgoing side
/// is told apart by whether [`Wire::request_tag`] is present, the incoming side by **[`Wire::reply_tag`] → [`Wire::stream_mark`] → anything else**,
/// in that order. ★That order is the contract★ — if one `In` gives `Some` for both, it is handled **as a reply first**
/// and [`crate::TransportEvent::AmbiguousRole`] is raised (a consumer contract violation signal).
///
/// ★Adding a field breaks every consumer★ — that is why the six are final for now.
pub trait Wire: Send + Sync + 'static {
    /// What I send.
    type Out: Send + 'static;
    /// What I receive.
    type In: Send + 'static;
    /// The pairing number. ★The contract is that the bound is only `Eq + Hash + Clone`★ — it requires neither order nor representation.
    type Tag: Eq + Hash + Clone + Send + Sync + 'static;
    /// The key that points at one stream.
    type StreamKey: Eq + Hash + Clone + Send + Sync + 'static;
    /// The decode failure reason. ★The crate puts only its classification ([`DecodeClass::class`]) into the event and **does not branch on it**★ — the value itself
    /// does not go up into the event (the reason is in [`DecodeClass`]).
    type DecodeError: DecodeClass + Send + 'static;

    fn encode(&self, out: &Self::Out) -> Frame;

    /// [`Frame::Keepalive`] does not come here — keepalive never meets the consumer vocabulary.
    fn decode(&self, frame: Frame) -> Result<Self::In, Self::DecodeError>;

    /// Whether the outgoing item waits for a reply. If `Some`, a waiting slot is created **before** sending.
    fn request_tag(&self, out: &Self::Out) -> Option<Self::Tag>;

    /// Whose reply the incoming item is. If `Some`, that slot is woken.
    fn reply_tag(&self, inn: &Self::In) -> Option<Self::Tag>;

    /// Whether the incoming item is a stream chunk.
    fn stream_mark(&self, inn: &Self::In) -> Option<StreamMark<Self::StreamKey>>;

    /// Builds a request that resumes a stream. ★The crate does not know its envelope and only knows "when" to call it★ — when it
    /// meets a gap and when the consumer called [`crate::Peer::resume_stream`].
    ///
    /// `after` means to send **from after that number**, and `None` means from the start of what the peer has.
    fn resume_request(&self, key: &Self::StreamKey, after: Option<u64>) -> Self::Out;
}

/// The classification of a decode failure — the only thing carried in [`crate::TransportEvent::DecodeFailed`] (TRD 3 §3-0 ⑤ · §5 19).
///
/// ★The point is that it is `&'static str`★ — a decoder error message easily quotes the received value (「invalid type: string "…"」)
/// and putting that in the event would let the peer plant text on the screen and the LLM surface. A fixed word blocks that route by type. The detailed
/// error is logged by the consumer into its own log inside [`Wire::decode`] (this crate has no logging).
pub trait DecodeClass {
    /// Which branch this failure is — for example `"json syntax"` · `"unknown tag"` · `"frame too short"`.
    fn class(&self) -> &'static str;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{TestIn, TestOut, TestWire};

    #[test]
    fn outbound_role_is_read_from_request_tag_alone() {
        let w = TestWire;
        assert_eq!(
            w.request_tag(&TestOut::Request {
                tag: 4,
                body: "x".into()
            }),
            Some(4)
        );
        assert_eq!(w.request_tag(&TestOut::Notice("x".into())), None);
        assert_eq!(
            w.request_tag(&TestOut::Resume {
                key: 1,
                after: None
            }),
            None
        );
    }

    #[test]
    fn inbound_roles_are_read_in_the_contracted_order() {
        let w = TestWire;
        let reply = TestIn::Reply {
            tag: 4,
            body: "ok".into(),
        };
        assert_eq!(w.reply_tag(&reply), Some(4));
        assert!(w.stream_mark(&reply).is_none());

        let chunk = TestIn::Chunk {
            key: 9,
            generation: 2,
            seq: 3,
            body: "b".into(),
        };
        assert!(w.reply_tag(&chunk).is_none());
        assert_eq!(
            w.stream_mark(&chunk).map(|m| (m.key, m.generation, m.seq)),
            Some((9, 2, 3))
        );

        let notice = TestIn::Notice("hi".into());
        assert!(w.reply_tag(&notice).is_none());
        assert!(w.stream_mark(&notice).is_none());
    }

    #[test]
    fn a_message_may_claim_both_roles_and_the_crate_can_see_it() {
        let w = TestWire;
        let both = TestIn::Ambiguous {
            tag: 1,
            key: 2,
            generation: 3,
            seq: 4,
        };
        assert!(w.reply_tag(&both).is_some());
        assert!(w.stream_mark(&both).is_some());
    }

    #[test]
    fn encode_decode_round_trips_both_frame_kinds() {
        let w = TestWire;
        let text = w.encode(&TestOut::Notice("hello".into()));
        assert!(matches!(text, Frame::Text(_)));
        let binary = w.encode(&TestOut::Blob(vec![1, 2, 3]));
        assert!(matches!(binary, Frame::Binary(_)));

        let decoded = w.decode(TestWire::encode_in(&TestIn::Notice("hello".into())));
        assert_eq!(decoded.unwrap(), TestIn::Notice("hello".into()));
    }

    #[test]
    fn decode_failure_surfaces_only_its_fixed_class() {
        let w = TestWire;
        let err = w
            .decode(Frame::Text("nonsense-from-the-peer".into()))
            .unwrap_err();
        assert!(!err.class().is_empty());
        assert!(!err.class().contains("nonsense-from-the-peer"));
    }

    #[test]
    fn resume_request_is_built_by_the_consumer() {
        let w = TestWire;
        assert_eq!(
            w.resume_request(&7, Some(42)),
            TestOut::Resume {
                key: 7,
                after: Some(42)
            }
        );
    }
}
