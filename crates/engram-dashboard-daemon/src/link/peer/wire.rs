//! PeerWire — the daemon ↔ daemon vocabulary: JSON text frames, one message per frame, tagged by `type`. Only daemons speak it, so it
//!   lives here and not in protocol (protocol is the shell ↔ daemon contract).
//!
//! What the peer link itself reads: `Hello` (the handshake — [`super::handshake`]), `Probe` / `ProbeAck` (proving a link alive for
//!   duplicate detection) and `Duplicate` (the structured duplicate-identity warning). Everything else — `Mail` and its `MailVerdict`,
//!   `ReplyOverdue`, the directory frames — is defined here with the shape the design fixes and handed on unread; the units that own them
//!   fill in the handling.
//!
//! Rules:
//! - ★No frame names an action other than mail★ (and directory / liveness housekeeping): a remote PC can do nothing else here by
//!   construction (ADR-0390). A new kind of action needs a new decision and a permission value beyond `mail`.
//! - ★A frame that does not decode is dropped and logged★, the link stays (the transport's `DecodeFailurePolicy::DropFrame`); binary and
//!   keepalive frames are not PeerWire.
//! - `Mail` is the only request (paired by its `id`, answered by `MailVerdict` with the same `id`); every other message is a push.
//! - ★The sender checks the encoded size before sending★ ([`encoded_len`] against [`MAX_FRAME_BYTES`]) — the link adapter refuses an
//!   oversized frame as a write error, which would drop the link.
//! - Directory frames carry no node field and `Mail` carries no sending node: whose frame it is comes only from the link's key.

// ADR-0388, ADR-0390

use std::convert::Infallible;
use std::fmt;

use engram_dashboard_base::text::hex_lower;
use engram_dashboard_transport::{DecodeClass, Frame, StreamMark, Wire};
use serde::{Deserialize, Serialize};

/// The PeerWire version in `Hello`; both sides must carry the same value.
pub(crate) const PEER_WIRE_VERSION: u32 = 1;

/// The ALPN of the peer relationship on the link adapter.
pub(crate) const ALPN: &[u8] = b"engram/peer/1";

/// Frame payload cap, both directions: a 64 KiB mail body escaped at worst (every byte a control character written `\u00XX`, 6×) is
///   384 KiB, plus room for the other fields.
pub(crate) const MAX_FRAME_BYTES: usize = 512 << 10;

/// A daemon run's identity — 128 random bits drawn at daemon start, sent in `Hello`. Two different instances behind one key at once are a
///   duplicated identity. Text = 32 lowercase hex digits.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Instance(u128);

impl Instance {
    /// A new random instance from the OS generator.
    pub(crate) fn generate() -> std::io::Result<Self> {
        let mut bytes = [0u8; 16];
        // getrandom 0.2's error is not a `std::error::Error` here — carried as text.
        getrandom::getrandom(&mut bytes).map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(Self(u128::from_be_bytes(bytes)))
    }

    #[cfg(test)]
    pub(crate) fn from_u128(value: u128) -> Self {
        Self(value)
    }

    fn to_text(self) -> String {
        hex_lower(&self.0.to_be_bytes())
    }

    fn parse(text: &str) -> Option<Self> {
        // Checked digit by digit: `from_str_radix` alone would also take a leading `+`.
        if text.len() != 32 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        u128::from_str_radix(text, 16).ok().map(Self)
    }
}

impl fmt::Debug for Instance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Instance({})", self.to_text())
    }
}

impl fmt::Display for Instance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_text())
    }
}

impl Serialize for Instance {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_text())
    }
}

impl<'de> Deserialize<'de> for Instance {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Instance::parse(&text)
            .ok_or_else(|| serde::de::Error::custom("an instance is 32 hex digits"))
    }
}

/// One PeerWire message.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum PeerMsg {
    /// Both directions — the dialer first; the acceptor answers once the dialer's `Hello` passed (the shell-link pattern).
    Hello {
        protocol_version: u32,
        instance: Instance,
    },
    /// The one request — mail for this PC's agents (rows for this PC only).
    Mail(Mail),
    /// The answer to a `Mail`, one row per request row.
    MailVerdict(MailVerdict),
    /// A reply contract this PC holds ran past its deadline — sent to the requester's PC, which renders its own notice.
    ReplyOverdue { request_id: String },
    /// The sender's whole agent directory, in parts above the frame cap; also the answer to `DirResync`.
    DirSnapshot(DirSnapshot),
    /// A change to the sender's directory.
    DirDelta(DirDelta),
    /// Asks the other PC for a fresh `DirSnapshot`.
    DirResync,
    /// Asks the other side to prove this link alive (duplicate detection).
    Probe { nonce: u64 },
    /// The answer to `Probe`, on the same link.
    ProbeAck { nonce: u64 },
    /// The structured duplicate-identity warning — pushed on the kept link and sent as a refusal's last frame.
    Duplicate { kind: DuplicateKind },
}

impl PeerMsg {
    /// The variant name — what logs and `Debug` show (bodies and names are agent content).
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::Hello { .. } => "Hello",
            Self::Mail(_) => "Mail",
            Self::MailVerdict(_) => "MailVerdict",
            Self::ReplyOverdue { .. } => "ReplyOverdue",
            Self::DirSnapshot(_) => "DirSnapshot",
            Self::DirDelta(_) => "DirDelta",
            Self::DirResync => "DirResync",
            Self::Probe { .. } => "Probe",
            Self::ProbeAck { .. } => "ProbeAck",
            Self::Duplicate { .. } => "Duplicate",
        }
    }
}

/// Prints the variant only.
impl fmt::Debug for PeerMsg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.kind())
    }
}

/// `Mail` — the sending node is not in it (stamped from the link); `from.name` is the sending daemon's claim.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Mail {
    pub(crate) id: String,
    pub(crate) from: MailFrom,
    /// At most 64 rows; `tag` = the sender's row index.
    pub(crate) rows: Vec<MailRow>,
    pub(crate) body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reply_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) request: Option<MailRequest>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MailFrom {
    pub(crate) agent_id: String,
    pub(crate) name: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MailRow {
    pub(crate) tag: u32,
    /// An agent name (without `@pc`) or id on the receiver.
    pub(crate) to: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MailRequest {
    pub(crate) reply_by_raw: String,
}

/// `MailVerdict` — rows matched to the request by `tag`, never by the address text.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MailVerdict {
    pub(crate) id: String,
    pub(crate) rows: Vec<VerdictRow>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct VerdictRow {
    pub(crate) tag: u32,
    pub(crate) outcome: Outcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) recipient_id: Option<String>,
    /// From a closed set the receiving side owns; the sender reads any other code as a refusal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) code: Option<String>,
    /// At most 512 bytes; shown labelled with the origin PC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Outcome {
    Delivered,
    Held,
    Refused,
    UnknownRecipient,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DirSnapshot {
    /// The publisher's run — its `Hello` instance.
    pub(crate) run: Instance,
    pub(crate) rev: u64,
    pub(crate) part: u32,
    pub(crate) last: bool,
    pub(crate) agents: Vec<DirAgent>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DirDelta {
    pub(crate) run: Instance,
    /// The previous `rev` + 1.
    pub(crate) rev: u64,
    pub(crate) upsert: Vec<DirAgent>,
    pub(crate) remove: Vec<String>,
}

/// One directory row — id, name and state only.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DirAgent {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) state: AgentState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AgentState {
    Live,
    Sleeping,
}

/// What a duplicate-identity warning says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DuplicateKind {
    /// Two copies with this identity were both proven running.
    Duplicate,
    /// One identity keeps replacing its own link — possibly a cloned copy carrying the same run.
    Suspected,
}

/// The frame text of `msg`. Serializing these types cannot fail (string keys, no custom serializers); the error is still carried.
pub(crate) fn encode_text(msg: &PeerMsg) -> Result<String, serde_json::Error> {
    serde_json::to_string(msg)
}

/// The frame of `msg` — for the handshake, which writes frames itself.
pub(crate) fn frame_of(msg: &PeerMsg) -> Frame {
    PeerWire.encode(msg)
}

/// The payload bytes `msg` takes on the link — a sender compares it with [`MAX_FRAME_BYTES`] before sending.
pub(crate) fn encoded_len(msg: &PeerMsg) -> Result<usize, serde_json::Error> {
    encode_text(msg).map(|text| text.len())
}

/// Reads a text frame as a message.
pub(crate) fn decode_text(text: &str) -> Result<PeerMsg, PeerDecodeError> {
    serde_json::from_str(text).map_err(|e| {
        // The serde message can quote the received text — only its class and place are logged.
        tracing::debug!(class = ?e.classify(), line = e.line(), column = e.column(), "PeerWire frame did not decode — dropped");
        PeerDecodeError::NotPeerWire
    })
}

/// Why a frame is not a PeerWire message — the class is all the transport reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PeerDecodeError {
    NotPeerWire,
    Binary,
    Keepalive,
}

impl DecodeClass for PeerDecodeError {
    fn class(&self) -> &'static str {
        match self {
            Self::NotPeerWire => "not a PeerWire message",
            Self::Binary => "binary frame",
            Self::Keepalive => "unexpected keepalive",
        }
    }
}

/// The peer link's [`Wire`] — stateless.
pub(crate) struct PeerWire;

impl Wire for PeerWire {
    type Out = PeerMsg;
    type In = PeerMsg;
    /// The mail id.
    type Tag = String;
    type StreamKey = Infallible;
    type DecodeError = PeerDecodeError;

    fn encode(&self, out: &PeerMsg) -> Frame {
        match encode_text(out) {
            Ok(text) => Frame::Text(text),
            Err(e) => {
                // Unreachable for these types; an empty frame is dropped by the receiver's decode and both sides log it.
                tracing::error!(
                    kind = out.kind(),
                    "PeerWire message could not be encoded: {e}"
                );
                Frame::Text(String::new())
            }
        }
    }

    fn decode(&self, frame: Frame) -> Result<PeerMsg, PeerDecodeError> {
        match frame {
            Frame::Text(text) => decode_text(&text),
            Frame::Binary(_) => Err(PeerDecodeError::Binary),
            Frame::Keepalive => Err(PeerDecodeError::Keepalive),
        }
    }

    fn request_tag(&self, out: &PeerMsg) -> Option<String> {
        match out {
            PeerMsg::Mail(mail) => Some(mail.id.clone()),
            _ => None,
        }
    }

    fn reply_tag(&self, inn: &PeerMsg) -> Option<String> {
        match inn {
            PeerMsg::MailVerdict(verdict) => Some(verdict.id.clone()),
            _ => None,
        }
    }

    fn stream_mark(&self, _inn: &PeerMsg) -> Option<StreamMark<Infallible>> {
        None
    }

    fn resume_request(&self, key: &Infallible, _after: Option<u64>) -> PeerMsg {
        match *key {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mail() -> PeerMsg {
        PeerMsg::Mail(Mail {
            id: "m-1".to_string(),
            from: MailFrom {
                agent_id: "a-1".to_string(),
                name: "writer".to_string(),
            },
            rows: vec![MailRow {
                tag: 0,
                to: "reader".to_string(),
            }],
            body: "hello".to_string(),
            reply_to: None,
            request: Some(MailRequest {
                reply_by_raw: "10m".to_string(),
            }),
        })
    }

    fn every_message() -> Vec<PeerMsg> {
        let agent = DirAgent {
            id: "a-1".to_string(),
            name: "writer".to_string(),
            state: AgentState::Sleeping,
        };
        vec![
            PeerMsg::Hello {
                protocol_version: PEER_WIRE_VERSION,
                instance: Instance::from_u128(u128::MAX - 7),
            },
            mail(),
            PeerMsg::MailVerdict(MailVerdict {
                id: "m-1".to_string(),
                rows: vec![VerdictRow {
                    tag: 0,
                    outcome: Outcome::UnknownRecipient,
                    recipient_id: None,
                    code: Some("REMOTE_REFUSED".to_string()),
                    reason: Some("no such agent".to_string()),
                }],
            }),
            PeerMsg::ReplyOverdue {
                request_id: "m-1".to_string(),
            },
            PeerMsg::DirSnapshot(DirSnapshot {
                run: Instance::from_u128(3),
                rev: 4,
                part: 0,
                last: true,
                agents: vec![agent.clone()],
            }),
            PeerMsg::DirDelta(DirDelta {
                run: Instance::from_u128(3),
                rev: 5,
                upsert: vec![agent],
                remove: vec!["a-2".to_string()],
            }),
            PeerMsg::DirResync,
            PeerMsg::Probe { nonce: u64::MAX },
            PeerMsg::ProbeAck { nonce: 1 },
            PeerMsg::Duplicate {
                kind: DuplicateKind::Suspected,
            },
        ]
    }

    #[test]
    fn every_message_round_trips_through_a_text_frame() {
        for msg in every_message() {
            let frame = PeerWire.encode(&msg);
            let Frame::Text(text) = &frame else {
                panic!("{msg:?} is not a text frame")
            };
            assert!(
                text.contains(&format!("\"type\":\"{}\"", msg.kind())),
                "{text}"
            );
            assert_eq!(PeerWire.decode(frame).unwrap(), msg);
        }
    }

    #[test]
    fn the_hello_carries_the_version_and_the_instance_as_text() {
        let text = encode_text(&PeerMsg::Hello {
            protocol_version: 1,
            instance: Instance::from_u128(0xabc),
        })
        .unwrap();
        assert_eq!(
            text,
            r#"{"type":"Hello","protocol_version":1,"instance":"00000000000000000000000000000abc"}"#
        );
    }

    #[test]
    fn instances_are_random_and_their_text_is_strict() {
        let a = Instance::generate().unwrap();
        let b = Instance::generate().unwrap();
        assert_ne!(a, b);
        assert_eq!(Instance::parse(&a.to_text()), Some(a));
        for bad in [
            "",
            "abc",
            "+0000000000000000000000000000abc",
            "0000000000000000000000000000000g",
        ] {
            assert_eq!(Instance::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn what_is_not_peer_wire_fails_to_decode_with_a_fixed_class() {
        let cases = [
            Frame::Text(r#"{"type":"Shutdown"}"#.to_string()),
            Frame::Text("not json at all".to_string()),
            Frame::Text(r#"{"type":"Hello","protocol_version":1,"instance":"x"}"#.to_string()),
            Frame::Binary(vec![1, 2]),
            Frame::Keepalive,
        ];
        for frame in cases {
            let err = PeerWire.decode(frame).unwrap_err();
            assert!(!err.class().contains("Shutdown"));
        }
    }

    #[test]
    fn only_mail_is_a_request_and_only_its_verdict_is_a_reply() {
        for msg in every_message() {
            let request = PeerWire.request_tag(&msg);
            let reply = PeerWire.reply_tag(&msg);
            match msg {
                PeerMsg::Mail(_) => assert_eq!(request.as_deref(), Some("m-1")),
                PeerMsg::MailVerdict(_) => assert_eq!(reply.as_deref(), Some("m-1")),
                _ => assert!(request.is_none() && reply.is_none(), "{msg:?}"),
            }
            assert!(PeerWire.stream_mark(&msg).is_none());
        }
    }

    #[test]
    fn debug_shows_the_variant_and_not_the_body() {
        let shown = format!("{:?}", mail());
        assert_eq!(shown, "Mail");
    }

    #[test]
    fn the_encoded_length_is_the_frame_payload() {
        let msg = mail();
        let Frame::Text(text) = PeerWire.encode(&msg) else {
            unreachable!()
        };
        assert_eq!(encoded_len(&msg).unwrap(), text.len());
        assert!(text.len() < MAX_FRAME_BYTES);
    }
}
