//! binary frame codec 의 golden(고정 바이트열) + roundtrip 회귀 테스트.
//! wire 포맷이 의도치 않게 바뀌면(헤더 순서/엔디언/오프셋) 즉시 깨지게 한다.

use engram_dashboard_protocol::{
    decode_frame, encode_structured_frame, encode_terminal_frame, CodecError, FRAME_HEADER_LEN,
    FRAME_TAG_STRUCTURED_EVENT, FRAME_TAG_TERMINAL_BYTES,
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

// ── ADR-0045 tag1 StructuredEvent ─────────────────────────────────────────────

/// tag1 golden: 헤더 레이아웃은 tag0 과 동일하고 첫 바이트만 0x01, payload 는 opaque 로 그대로 실린다.
#[test]
fn golden_structured_frame() {
    let id = Uuid::from_bytes([
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f,
    ]);
    let epoch: u32 = 7;
    let seq: u64 = 0x0102_0304_0506_0708;
    // codec 은 payload 스키마 무지 — 임의 바이트(직렬화된 이벤트를 흉내)를 그대로 통과시켜야 한다.
    let payload = br#"{"TextDelta":"hi"}"#;

    let frame = encode_structured_frame(id, epoch, seq, payload);

    let mut expected = Vec::new();
    expected.push(FRAME_TAG_STRUCTURED_EVENT); // 0x01
    expected.extend_from_slice(&[
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f,
    ]); // id
    expected.extend_from_slice(&[0x00, 0x00, 0x00, 0x07]); // epoch BE
    expected.extend_from_slice(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]); // seq BE
    expected.extend_from_slice(payload); // payload opaque

    assert_eq!(
        frame, expected,
        "tag1 frame 바이트 레이아웃이 golden 과 불일치"
    );
    assert_eq!(frame.len(), FRAME_HEADER_LEN + payload.len());
}

/// tag1 encode→decode round-trip: 헤더(agent_id/epoch/seq)가 정확히 복원되고 payload 가
/// opaque 로 **한 바이트도 손실·해석 없이** 보존되는지. codec 이 스키마를 모른다는 불변식의 핵심 검증.
#[test]
fn roundtrip_structured_frame() {
    let id = Uuid::new_v4();
    // 4096 버퍼 경계를 넘기고 non-UTF8/제어바이트 포함 — codec 이 내용을 건드리지 않음을 강제.
    let payload: Vec<u8> = (0u8..=255).cycle().take(5000).collect();
    let frame = encode_structured_frame(id, 42, 999_999, &payload);

    let decoded = decode_frame(&frame).expect("tag1 decode 성공");
    assert_eq!(decoded.tag, FRAME_TAG_STRUCTURED_EVENT);
    assert_eq!(decoded.agent_id, id, "헤더 agent_id 정확 파싱");
    assert_eq!(decoded.epoch, 42, "헤더 epoch 정확 파싱");
    assert_eq!(decoded.seq, 999_999, "헤더 seq 정확 파싱");
    assert_eq!(decoded.payload, &payload[..], "payload opaque 무손실 보존");
}

/// tag≥2 는 여전히 거부(TRD §5 unknown tag). tag1 통과가 게이트를 과하게 열지 않았는지 확인.
#[test]
fn tag_two_is_still_unknown() {
    let mut buf = vec![0u8; FRAME_HEADER_LEN];
    buf[0] = 2; // 아직 정의 안 된 tag
    assert_eq!(decode_frame(&buf), Err(CodecError::UnknownTag(2)));
}

// ── 경계값·최소길이 회귀 (adversary FIX) ──────────────────────────────────────

/// epoch/seq 극값 round-trip: BE 변환이 상한(u32::MAX / u64::MAX)에서도 무손실인지.
/// to_be_bytes/from_be_bytes 오프셋·자릿수 실수를 경계에서 잡는다(tag0·tag1 둘 다).
#[test]
fn roundtrip_epoch_seq_max() {
    let id = Uuid::new_v4();
    for encode in [
        encode_terminal_frame as fn(Uuid, u32, u64, &[u8]) -> Vec<u8>,
        encode_structured_frame,
    ] {
        let frame = encode(id, u32::MAX, u64::MAX, b"edge");
        let decoded = decode_frame(&frame).expect("경계값 decode 성공");
        assert_eq!(decoded.epoch, u32::MAX, "epoch=u32::MAX 무손실");
        assert_eq!(decoded.seq, u64::MAX, "seq=u64::MAX 무손실");
        assert_eq!(decoded.agent_id, id);
        assert_eq!(decoded.payload, b"edge");
    }
}

/// tag1 정확히 29바이트(헤더만, 빈 payload) encode→decode. 최소 유효 프레임 경계.
#[test]
fn structured_empty_payload_is_min_valid() {
    let id = Uuid::new_v4();
    let frame = encode_structured_frame(id, 3, 5, &[]);
    assert_eq!(
        frame.len(),
        FRAME_HEADER_LEN,
        "빈 payload = 정확히 헤더 길이"
    );
    let decoded = decode_frame(&frame).expect("29바이트 tag1 decode 성공");
    assert_eq!(decoded.tag, FRAME_TAG_STRUCTURED_EVENT);
    assert_eq!(decoded.epoch, 3);
    assert_eq!(decoded.seq, 5);
    assert_eq!(decoded.payload.len(), 0);
}

/// 헤더 미만 입력(0·1·28바이트)은 패닉이 아니라 TooShort 에러로 안전 반환.
/// 특히 1바이트(tag만)·28바이트(헤더-1)에서 슬라이스 인덱싱이 패닉하지 않아야 한다.
#[test]
fn short_inputs_return_too_short_not_panic() {
    for len in [0usize, 1, FRAME_HEADER_LEN - 1] {
        let buf = vec![0u8; len];
        assert_eq!(
            decode_frame(&buf),
            Err(CodecError::TooShort { len }),
            "{len}바이트 입력은 TooShort"
        );
    }
}
