//! Golden (fixed byte sequence) + roundtrip regression tests of the binary frame codec.
//! They break immediately if the wire format changes unintentionally (header order/endianness/offsets).

use engram_dashboard_protocol::{
    decode_frame, encode_structured_frame, encode_terminal_frame, peek_frame_header, CodecError,
    FrameHeader, FRAME_HEADER_LEN, FRAME_TAG_STRUCTURED_EVENT, FRAME_TAG_TERMINAL_BYTES,
};
use uuid::Uuid;

/// Known input → exact byte sequence. Header layout `[tag][id:16][epoch:4 BE][seq:8 BE][payload]`.
#[test]
fn golden_terminal_frame() {
    // Deterministic UUID (16 bytes 0x00..0x0f).
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
    let payload: Vec<u8> = (0u8..=255).cycle().take(5000).collect(); // crosses the 4096 buffer boundary
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
    buf[0] = 0xFF; // unknown tag
    assert_eq!(decode_frame(&buf), Err(CodecError::UnknownTag(0xFF)));
}

// ── ADR-0045 tag1 StructuredEvent ─────────────────────────────────────────────

/// tag1 golden: the header layout is the same as tag0 with only the first byte 0x01, and the payload is carried as is, opaque.
#[test]
fn golden_structured_frame() {
    let id = Uuid::from_bytes([
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f,
    ]);
    let epoch: u32 = 7;
    let seq: u64 = 0x0102_0304_0506_0708;
    // The codec is ignorant of the payload schema — arbitrary bytes (imitating a serialized event) must pass through as is.
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

/// tag1 encode→decode round-trip: whether the header (agent_id/epoch/seq) is restored exactly and the payload is preserved
/// opaque with **not a single byte lost or interpreted**. The core check of the invariant that the codec does not know the schema.
#[test]
fn roundtrip_structured_frame() {
    let id = Uuid::new_v4();
    // Crosses the 4096 buffer boundary and includes non-UTF8/control bytes — enforces that the codec does not touch the content.
    let payload: Vec<u8> = (0u8..=255).cycle().take(5000).collect();
    let frame = encode_structured_frame(id, 42, 999_999, &payload);

    let decoded = decode_frame(&frame).expect("tag1 decode 성공");
    assert_eq!(decoded.tag, FRAME_TAG_STRUCTURED_EVENT);
    assert_eq!(decoded.agent_id, id, "헤더 agent_id 정확 파싱");
    assert_eq!(decoded.epoch, 42, "헤더 epoch 정확 파싱");
    assert_eq!(decoded.seq, 999_999, "헤더 seq 정확 파싱");
    assert_eq!(decoded.payload, &payload[..], "payload opaque 무손실 보존");
}

/// tag≥2 is still rejected (TRD §5 unknown tag). Checks that letting tag1 through did not open the gate too far.
#[test]
fn tag_two_is_still_unknown() {
    let mut buf = vec![0u8; FRAME_HEADER_LEN];
    buf[0] = 2; // a tag not yet defined
    assert_eq!(decode_frame(&buf), Err(CodecError::UnknownTag(2)));
}

// ── boundary values · minimum length regression ──────────────────────────────────

/// epoch/seq extreme round-trip: whether the BE conversion is lossless even at the upper bounds (u32::MAX / u64::MAX).
/// Catches to_be_bytes/from_be_bytes offset and digit mistakes at the boundary (both tag0 and tag1).
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

/// tag1 exactly 29 bytes (header only, empty payload) encode→decode. The minimum valid frame boundary.
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

/// Input shorter than the header (0·1·28 bytes) is returned safely as a TooShort error, not a panic.
/// In particular, slice indexing must not panic at 1 byte (tag only) and 28 bytes (header minus 1).
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

// ── ADR-0231 header field reading — does not tell tags apart ─────────────────────

/// A frame `decode_frame` rejected on an unknown tag still has its header fields read — the shell makes a placeholder with that seq.
#[test]
fn peek_reads_the_header_of_an_unknown_tag_frame() {
    let id = Uuid::new_v4();
    let mut frame = encode_structured_frame(id, 9, 0x0102_0304_0506_0708, b"future");
    frame[0] = 0x07; // a tag not yet defined
    assert_eq!(decode_frame(&frame), Err(CodecError::UnknownTag(0x07)));
    assert_eq!(
        peek_frame_header(&frame),
        Ok(FrameHeader {
            tag: 0x07,
            agent_id: id,
            epoch: 9,
            seq: 0x0102_0304_0506_0708,
        })
    );
}

/// Shorter than the header length, peek is also TooShort — the only shape where the seq cannot be read and so no placeholder can be made.
#[test]
fn peek_rejects_only_short_inputs() {
    for len in [0usize, 1, FRAME_HEADER_LEN - 1] {
        assert_eq!(
            peek_frame_header(&vec![0xFFu8; len]),
            Err(CodecError::TooShort { len }),
            "{len}바이트 입력은 TooShort"
        );
    }
    assert!(peek_frame_header(&[0xFFu8; FRAME_HEADER_LEN]).is_ok());
}
