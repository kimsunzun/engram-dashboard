//! Binary frame codec of the output hot path (design §1-2).
//!
//! Header: `[tag:1][agent_id:16][epoch:4 BE][seq:8 BE][raw payload...]`; sent as is on the WS binary opcode (no base64-in-JSON, no serde
//!   parsing).
//! Multi-byte values are big-endian; JS decodes with `DataView.getUint32(false)`/`getBigUint64(false)`.

use crate::ids::AgentId;

/// payload = VT byte stream (console).
pub const FRAME_TAG_TERMINAL_BYTES: u8 = 0;

/// ADR-0045. payload = **self-describing serialized structured event**, opaque to the codec (format and event type belong to the daemon adapter, B7).
/// Same header layout as tag0.
pub const FRAME_TAG_STRUCTURED_EVENT: u8 = 1;

pub const FRAME_HEADER_LEN: usize = 1 + 16 + 4 + 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    TooShort { len: usize },
    UnknownTag(u8),
}

impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodecError::TooShort { len } => {
                write!(f, "frame too short: {len} < {FRAME_HEADER_LEN}")
            }
            CodecError::UnknownTag(t) => write!(f, "unknown frame tag: {t}"),
        }
    }
}

impl std::error::Error for CodecError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedFrame<'a> {
    pub tag: u8,
    pub agent_id: AgentId,
    pub epoch: u32,
    pub seq: u64,
    pub payload: &'a [u8],
}

fn encode_frame(tag: u8, agent_id: AgentId, epoch: u32, seq: u64, payload: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(FRAME_HEADER_LEN + payload.len());
    buf.push(tag);
    buf.extend_from_slice(agent_id.as_bytes()); // 16 bytes, RFC4122 network order
    buf.extend_from_slice(&epoch.to_be_bytes());
    buf.extend_from_slice(&seq.to_be_bytes());
    buf.extend_from_slice(payload);
    buf
}

pub fn encode_terminal_frame(agent_id: AgentId, epoch: u32, seq: u64, payload: &[u8]) -> Vec<u8> {
    encode_frame(FRAME_TAG_TERMINAL_BYTES, agent_id, epoch, seq, payload)
}

pub fn encode_structured_frame(agent_id: AgentId, epoch: u32, seq: u64, payload: &[u8]) -> Vec<u8> {
    encode_frame(FRAME_TAG_STRUCTURED_EVENT, agent_id, epoch, seq, payload)
}

/// Header fields (tag · agent id · incarnation marker · seq) — read without looking at the payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    pub tag: u8,
    pub agent_id: AgentId,
    pub epoch: u32,
    pub seq: u64,
}

/// Reads only the header fields — **does not tell tags apart**; the only rejection is a too-short header (`TooShort`).
///
/// ★A frame [`decode_frame`] rejected on an unknown tag is still readable here (ADR-0231)★ — the fixed-position fields let the shell build a same-seq
/// [`crate::placeholder_error_frame`].
// ADR-0231
pub fn peek_frame_header(buf: &[u8]) -> Result<FrameHeader, CodecError> {
    if buf.len() < FRAME_HEADER_LEN {
        return Err(CodecError::TooShort { len: buf.len() });
    }
    let mut id_bytes = [0u8; 16];
    id_bytes.copy_from_slice(&buf[1..17]);
    let agent_id = AgentId::from_bytes(id_bytes);

    let mut epoch_bytes = [0u8; 4];
    epoch_bytes.copy_from_slice(&buf[17..21]);
    let epoch = u32::from_be_bytes(epoch_bytes);

    let mut seq_bytes = [0u8; 8];
    seq_bytes.copy_from_slice(&buf[21..29]);
    let seq = u64::from_be_bytes(seq_bytes);

    Ok(FrameHeader {
        tag: buf[0],
        agent_id,
        epoch,
        seq,
    })
}

/// Unknown tags (≥2) keep being rejected so the client relay does not pass on unsupported frames.
pub fn decode_frame(buf: &[u8]) -> Result<DecodedFrame<'_>, CodecError> {
    let header = peek_frame_header(buf)?;
    if header.tag != FRAME_TAG_TERMINAL_BYTES && header.tag != FRAME_TAG_STRUCTURED_EVENT {
        return Err(CodecError::UnknownTag(header.tag));
    }
    Ok(DecodedFrame {
        tag: header.tag,
        agent_id: header.agent_id,
        epoch: header.epoch,
        seq: header.seq,
        payload: &buf[FRAME_HEADER_LEN..],
    })
}
