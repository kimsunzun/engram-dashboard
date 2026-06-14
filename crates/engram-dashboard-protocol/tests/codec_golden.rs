//! binary frame codec 의 golden(고정 바이트열) + roundtrip 회귀 테스트.
//! wire 포맷이 의도치 않게 바뀌면(헤더 순서/엔디언/오프셋) 즉시 깨지게 한다.

use engram_dashboard_protocol::{
    decode_frame, encode_terminal_frame, CodecError, FRAME_HEADER_LEN, FRAME_TAG_TERMINAL_BYTES,
};
use uuid::Uuid;

/// 알려진 입력 → 정확한 바이트열. 헤더 레이아웃 `[tag][id:16][epoch:4 BE][seq:8 BE][payload]`.
#[test]
fn golden_terminal_frame() {
    // 결정적 UUID(16바이트 0x00..0x0f).
    let id = Uuid::from_bytes([
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f,
    ]);
    let epoch: u32 = 7;
    let seq: u64 = 0x0102_0304_0506_0708;
    let payload = b"hi";

    let frame = encode_terminal_frame(id, epoch, seq, payload);

    let mut expected = Vec::new();
    expected.push(FRAME_TAG_TERMINAL_BYTES); // 0x00
    expected.extend_from_slice(&[
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f,
    ]); // id
    expected.extend_from_slice(&[0x00, 0x00, 0x00, 0x07]); // epoch BE
    expected.extend_from_slice(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]); // seq BE
    expected.extend_from_slice(b"hi"); // payload

    assert_eq!(frame, expected, "frame 바이트 레이아웃이 golden 과 불일치");
    assert_eq!(frame.len(), FRAME_HEADER_LEN + 2);
}

#[test]
fn roundtrip_terminal_frame() {
    let id = Uuid::new_v4();
    let payload: Vec<u8> = (0u8..=255).cycle().take(5000).collect(); // 4096 버퍼 경계 넘김
    let frame = encode_terminal_frame(id, 42, 999_999, &payload);

    let decoded = decode_frame(&frame).expect("decode 성공");
    assert_eq!(decoded.tag, FRAME_TAG_TERMINAL_BYTES);
    assert_eq!(decoded.agent_id, id);
    assert_eq!(decoded.epoch, 42);
    assert_eq!(decoded.seq, 999_999);
    assert_eq!(decoded.payload, &payload[..]);
}

#[test]
fn empty_payload_is_valid() {
    let id = Uuid::new_v4();
    let frame = encode_terminal_frame(id, 0, 0, &[]);
    assert_eq!(frame.len(), FRAME_HEADER_LEN);
    let decoded = decode_frame(&frame).expect("빈 payload 도 유효");
    assert_eq!(decoded.payload.len(), 0);
    assert_eq!(decoded.seq, 0);
}

#[test]
fn too_short_is_error() {
    let buf = [0u8; FRAME_HEADER_LEN - 1];
    assert_eq!(
        decode_frame(&buf),
        Err(CodecError::TooShort {
            len: FRAME_HEADER_LEN - 1
        })
    );
}

#[test]
fn unknown_tag_is_error() {
    let mut buf = vec![0u8; FRAME_HEADER_LEN];
    buf[0] = 0xFF; // 알 수 없는 tag
    assert_eq!(decode_frame(&buf), Err(CodecError::UnknownTag(0xFF)));
}
