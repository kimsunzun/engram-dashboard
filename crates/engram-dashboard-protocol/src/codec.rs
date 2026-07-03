//! output hot path 의 binary frame codec(설계 §1-2).
//!
//! 고정 헤더: `[tag:1][agent_id:16][epoch:4 BE][seq:8 BE][raw payload...]`.
//! base64-in-JSON(33% 팽창)·serde 파싱 0 — WS binary opcode 로 그대로 전송.
//! 멀티바이트는 빅엔디언(네트워크 바이트 오더, 모바일/타 플랫폼 정합). JS 는
//! `DataView.getUint32(false)`/`getBigUint64(false)` 로 디코드.

use crate::ids::AgentId;

/// tag=0 = TerminalBytes(VT 바이트 스트림 — 콘솔).
pub const FRAME_TAG_TERMINAL_BYTES: u8 = 0;

/// tag=1 = StructuredEvent(ADR-0045). payload = **self-describing 직렬화 구조화 이벤트**.
/// codec 은 payload 스키마를 모른다(opaque 바이트) — 직렬화 형식·이벤트 타입은 daemon adapter(B7)
/// 소관이고, 여기선 헤더만 붙이고 payload 는 그대로 실어 보낸다. tag0 과 헤더 레이아웃 동일.
pub const FRAME_TAG_STRUCTURED_EVENT: u8 = 1;

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

/// tag + 고정 헤더 + payload 인코드(내부 공통). tag0/tag1 은 payload 의미만 다르고
/// 헤더 레이아웃은 동일하므로 한 헬퍼로 묶는다. codec 은 payload 내용을 해석하지 않는다.
fn encode_frame(tag: u8, agent_id: AgentId, epoch: u32, seq: u64, payload: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(FRAME_HEADER_LEN + payload.len());
    buf.push(tag);
    buf.extend_from_slice(agent_id.as_bytes()); // 16 bytes, RFC4122 network order
    buf.extend_from_slice(&epoch.to_be_bytes()); // 4
    buf.extend_from_slice(&seq.to_be_bytes()); // 8
    buf.extend_from_slice(payload);
    buf
}

/// TerminalBytes 프레임 인코드(tag0). 헤더 + raw 바이트(복사 1회 — 전송 버퍼 생성).
pub fn encode_terminal_frame(agent_id: AgentId, epoch: u32, seq: u64, payload: &[u8]) -> Vec<u8> {
    encode_frame(FRAME_TAG_TERMINAL_BYTES, agent_id, epoch, seq, payload)
}

/// StructuredEvent 프레임 인코드(tag1, ADR-0045). payload = 이미 직렬화된 구조화 이벤트 바이트.
/// codec 은 payload 를 opaque 로 취급(스키마 무지) — 어떤 형식으로 직렬화할지는 호출자(daemon adapter).
pub fn encode_structured_frame(agent_id: AgentId, epoch: u32, seq: u64, payload: &[u8]) -> Vec<u8> {
    encode_frame(FRAME_TAG_STRUCTURED_EVENT, agent_id, epoch, seq, payload)
}

/// 임의 프레임 디코드. 헤더 파싱 후 payload 슬라이스(zero-copy 빌림) 반환.
///
/// ★ADR-0045★: known tag(0=Terminal, 1=Structured)는 **헤더만 파싱하고 payload 는 opaque
/// 슬라이스로 반환**한다 — codec 은 tag1 payload 의 이벤트 스키마를 해석하지 않는다(재사용·교체성).
/// unknown tag(≥2)는 계속 거부해 클라 relay 가 미지원 프레임을 흘리지 않게 한다.
pub fn decode_frame(buf: &[u8]) -> Result<DecodedFrame<'_>, CodecError> {
    if buf.len() < FRAME_HEADER_LEN {
        return Err(CodecError::TooShort { len: buf.len() });
    }
    let tag = buf[0];
    if tag != FRAME_TAG_TERMINAL_BYTES && tag != FRAME_TAG_STRUCTURED_EVENT {
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
