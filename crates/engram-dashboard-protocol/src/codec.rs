//! output hot path 의 binary frame codec(설계 §1-2).
//!
//! 고정 헤더: `[tag:1][agent_id:16][epoch:4 BE][seq:8 BE][raw payload...]`.
//! base64-in-JSON(33% 팽창)·serde 파싱 0 — WS binary opcode 로 그대로 전송.
//! 멀티바이트는 빅엔디언(네트워크 바이트 오더, 모바일/타 플랫폼 정합). JS 는
//! `DataView.getUint32(false)`/`getBigUint64(false)` 로 디코드.

use crate::ids::AgentId;

/// tag=0 = TerminalBytes(현 유일). 미래 binary 출력 variant 는 다음 tag 로 확장.
pub const FRAME_TAG_TERMINAL_BYTES: u8 = 0;

/// 헤더 길이: tag(1)+agent_id(16)+epoch(4)+seq(8) = 29 바이트.
pub const FRAME_HEADER_LEN: usize = 1 + 16 + 4 + 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    /// 헤더보다 짧음.
    TooShort { len: usize },
    /// 알 수 없는 tag.
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

/// 디코드 결과. payload 는 입력 버퍼를 빌려(zero-copy) 가리킨다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedFrame<'a> {
    pub tag: u8,
    pub agent_id: AgentId,
    pub epoch: u32,
    pub seq: u64,
    pub payload: &'a [u8],
}

/// TerminalBytes 프레임 인코드. 헤더 + raw 바이트(복사 1회 — 전송 버퍼 생성).
pub fn encode_terminal_frame(agent_id: AgentId, epoch: u32, seq: u64, payload: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(FRAME_HEADER_LEN + payload.len());
    buf.push(FRAME_TAG_TERMINAL_BYTES);
    buf.extend_from_slice(agent_id.as_bytes()); // 16 bytes, RFC4122 network order
    buf.extend_from_slice(&epoch.to_be_bytes()); // 4
    buf.extend_from_slice(&seq.to_be_bytes()); // 8
    buf.extend_from_slice(payload);
    buf
}

/// 임의 프레임 디코드. 헤더 파싱 후 payload 슬라이스 반환.
pub fn decode_frame(buf: &[u8]) -> Result<DecodedFrame<'_>, CodecError> {
    if buf.len() < FRAME_HEADER_LEN {
        return Err(CodecError::TooShort { len: buf.len() });
    }
    let tag = buf[0];
    if tag != FRAME_TAG_TERMINAL_BYTES {
        return Err(CodecError::UnknownTag(tag));
    }
    // 헤더 슬라이스 → 고정 배열 변환은 길이 검사 후라 안전(unwrap 안 씀, 명시 배열).
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
