//! output hot path 의 binary frame codec(설계 §1-2).
//!
//! 고정 헤더: `[tag:1][agent_id:16][epoch:4 BE][seq:8 BE][raw payload...]`.
//! base64-in-JSON(33% 팽창)·serde 파싱 0 — WS binary opcode 로 그대로 전송.
//! 멀티바이트는 빅엔디언(네트워크 바이트 오더, 모바일/타 플랫폼 정합). JS 는
//! `DataView.getUint32(false)`/`getBigUint64(false)` 로 디코드.

use crate::ids::AgentId;

/// payload = VT 바이트 스트림(콘솔).
pub const FRAME_TAG_TERMINAL_BYTES: u8 = 0;

/// ADR-0045. payload = **self-describing 직렬화 구조화 이벤트**.
/// codec 은 payload 스키마를 모른다(opaque 바이트) — 직렬화 형식·이벤트 타입은 daemon adapter(B7)
/// 소관이고, 여기선 헤더만 붙이고 payload 는 그대로 실어 보낸다. tag0 과 헤더 레이아웃 동일.
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

/// unknown tag(≥2)는 계속 거부해 클라 relay 가 미지원 프레임을 흘리지 않게 한다.
pub fn decode_frame(buf: &[u8]) -> Result<DecodedFrame<'_>, CodecError> {
    if buf.len() < FRAME_HEADER_LEN {
        return Err(CodecError::TooShort { len: buf.len() });
    }
    let tag = buf[0];
    if tag != FRAME_TAG_TERMINAL_BYTES && tag != FRAME_TAG_STRUCTURED_EVENT {
        return Err(CodecError::UnknownTag(tag));
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

    Ok(DecodedFrame {
        tag,
        agent_id,
        epoch,
        seq,
        payload: &buf[FRAME_HEADER_LEN..],
    })
}
