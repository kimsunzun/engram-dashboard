//! The daemon ↔ shell packet vocabulary as a transport [`Wire`] — raw pass-through (TRD S21 trd-3 §3-3 「DaemonWire 모양」).
//!
//! - In = [`RawIn`] · Out = [`RawOut`] — text and binary frames byte for byte. Decoding stays in the upper layer
//!   (`agent_conn` reads the command JSON), so this wire never fails to decode and keeps three behaviours a decoding
//!   wire would drop: an `Error` reply to a broken command with the connection kept, 「already authenticated」 on a
//!   second `Auth`, and `Error` + close on a binary frame.
//! - ★No pairing and no stream layer★ — `request_tag` · `reply_tag` · `stream_mark` are `None` for every value. A
//!   `Some` reply tag would make the transport look the frame up in its waiting table and swallow it as
//!   `UnmatchedReply` (the daemon never waits on a request). Tag and stream key are [`Infallible`], so the crate can
//!   build neither.
// ADR-0310

use std::convert::Infallible;
use std::fmt;

use engram_dashboard_transport::{DecodeClass, Frame, StreamMark, Wire};

/// What the daemon receives from a client.
pub enum RawIn {
    Text(String),
    Binary(Vec<u8>),
}

/// What the daemon sends to a client.
pub enum RawOut {
    Text(String),
    Binary(Vec<u8>),
}

/// Prints the variant and the byte length only — frames carry agent payloads and, before auth, a token.
impl fmt::Debug for RawIn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (name, len) = match self {
            RawIn::Text(text) => ("Text", text.len()),
            RawIn::Binary(bytes) => ("Binary", bytes.len()),
        };
        f.debug_struct(name).field("bytes", &len).finish()
    }
}

/// The only decode failure — a keepalive handed to [`Wire::decode`], which the transport never does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DaemonDecodeError;

impl DecodeClass for DaemonDecodeError {
    fn class(&self) -> &'static str {
        "unexpected keepalive"
    }
}

/// The daemon's [`Wire`] — stateless, one per registry.
pub struct DaemonWire;

impl Wire for DaemonWire {
    type Out = RawOut;
    type In = RawIn;
    type Tag = Infallible;
    type StreamKey = Infallible;
    type DecodeError = DaemonDecodeError;

    fn encode(&self, out: &RawOut) -> Frame {
        match out {
            RawOut::Text(text) => Frame::Text(text.clone()),
            RawOut::Binary(bytes) => Frame::Binary(bytes.clone()),
        }
    }

    fn decode(&self, frame: Frame) -> Result<RawIn, DaemonDecodeError> {
        match frame {
            Frame::Text(text) => Ok(RawIn::Text(text)),
            Frame::Binary(bytes) => Ok(RawIn::Binary(bytes)),
            Frame::Keepalive => Err(DaemonDecodeError),
        }
    }

    fn request_tag(&self, _out: &RawOut) -> Option<Infallible> {
        None
    }

    fn reply_tag(&self, _inn: &RawIn) -> Option<Infallible> {
        None
    }

    fn stream_mark(&self, _inn: &RawIn) -> Option<StreamMark<Infallible>> {
        None
    }

    fn resume_request(&self, key: &Infallible, _after: Option<u64>) -> RawOut {
        match *key {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_and_binary_pass_through_byte_for_byte_in_both_directions() {
        let w = DaemonWire;
        assert_eq!(
            w.encode(&RawOut::Text("{\"Hello\":{}}".into())),
            Frame::Text("{\"Hello\":{}}".into())
        );
        assert_eq!(
            w.encode(&RawOut::Binary(vec![0xff, 0, 7])),
            Frame::Binary(vec![0xff, 0, 7])
        );
        // Not JSON on purpose — the wire does not judge text; the upper layer answers a broken command.
        match w.decode(Frame::Text("nonsense".into())).unwrap() {
            RawIn::Text(text) => assert_eq!(text, "nonsense"),
            other => panic!("{other:?}"),
        }
        match w.decode(Frame::Binary(vec![1, 2, 3])).unwrap() {
            RawIn::Binary(bytes) => assert_eq!(bytes, vec![1, 2, 3]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn nothing_pairs_and_nothing_is_a_stream_fragment() {
        let w = DaemonWire;
        assert!(w.request_tag(&RawOut::Text("x".into())).is_none());
        for inn in [
            RawIn::Text("{\"Ack\":{}}".into()),
            RawIn::Binary(vec![0; 32]),
        ] {
            assert!(w.reply_tag(&inn).is_none());
            assert!(w.stream_mark(&inn).is_none());
        }
    }

    #[test]
    fn debug_shows_the_length_not_the_body() {
        let shown = format!("{:?}", RawIn::Text(r#"{"Auth":{"token":"SECRET"}}"#.into()));
        assert!(!shown.contains("SECRET"), "{shown}");
        assert!(shown.contains("bytes"), "{shown}");
    }
}
