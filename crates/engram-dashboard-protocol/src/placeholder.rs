//! Output frame placeholder — does not leave empty the seq of an event that could not be carried (ADR-0231).
//!
//! ★Why it exists (load-bearing)★: in live mode a view holds `seq > 마지막+1` (seq greater than last+1) with no deadline and releases it only when `마지막+1` (last+1) arrives.
//! So if the daemon sink or the shell relay silently drops one seq, the frames after it pile up to the hold cap — and
//! for an idle agent, forever — and that view stalls. The dropping spot sends this frame **with the same seq** instead: a tag1
//! `StructuredEvent::Error` with a fixed wording. The chat view draws one error row and the terminal view ignores tag1 but the seq
//! advances.
//!
//! ★The daemon sink and the shell share one set★ — the placeholders from the two places must match down to the bytes so the view cannot tell
//! the source (a test measures that identity).
//! ★The wording is a fixed string and the payload is frozen at compile time★ — the placeholder stands in for a dropped event,
//! so it must have no room for its own serialization to fail. So it carries a constant instead of runtime serialization, and a test
//! guards that the constant equals the serialization of `StructuredEvent::Error`.

use crate::codec::encode_structured_frame;
use crate::ids::AgentId;

// The single place for the wording literal — the public constant and the payload constant are both expanded from here (`concat!` accepts only literals).
macro_rules! placeholder_message {
    () => {
        "이 출력 사건을 싣지 못했습니다"
    };
}

/// Wording of the placeholder `Error`. A human-readable opaque string — consumers do not branch on this value.
pub const PLACEHOLDER_ERROR_MESSAGE: &str = placeholder_message!();

/// Exactly the JSON serialization of `StructuredEvent::Error { message: PLACEHOLDER_ERROR_MESSAGE }`.
const PLACEHOLDER_ERROR_PAYLOAD: &str = concat!(
    r#"{"type":"Error","message":""#,
    placeholder_message!(),
    r#""}"#
);

/// A tag1 placeholder frame carrying `agent_id`·`epoch`·`seq`.
// ADR-0231
pub fn placeholder_error_frame(agent_id: AgentId, epoch: u32, seq: u64) -> Vec<u8> {
    encode_structured_frame(agent_id, epoch, seq, PLACEHOLDER_ERROR_PAYLOAD.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{decode_frame, FRAME_TAG_STRUCTURED_EVENT};
    use crate::messages::StructuredEvent;

    // The constant payload is hand-written JSON, so it can drift from the wire type — this catches that drift in both directions.
    #[test]
    fn the_payload_is_the_serialized_error_event() {
        let event = StructuredEvent::Error {
            message: PLACEHOLDER_ERROR_MESSAGE.to_owned(),
        };
        assert_eq!(
            PLACEHOLDER_ERROR_PAYLOAD.as_bytes(),
            serde_json::to_vec(&event).expect("직렬화").as_slice(),
            "상수 페이로드 = StructuredEvent::Error 의 직렬화"
        );
        let parsed: StructuredEvent =
            serde_json::from_str(PLACEHOLDER_ERROR_PAYLOAD).expect("wire 타입으로 읽힌다");
        assert_eq!(parsed, event);
    }

    #[test]
    fn the_frame_carries_the_given_header_as_tag1() {
        let agent = AgentId::from_u128(0x0102_0304_0506_0708_090a_0b0c_0d0e_0f10);
        let frame = placeholder_error_frame(agent, 7, 42);
        let decoded = decode_frame(&frame).expect("정상 tag1 프레임");
        assert_eq!(decoded.tag, FRAME_TAG_STRUCTURED_EVENT);
        assert_eq!(decoded.agent_id, agent);
        assert_eq!(decoded.epoch, 7);
        assert_eq!(decoded.seq, 42, "버려진 사건의 seq 를 그대로 단다");
        assert_eq!(decoded.payload, PLACEHOLDER_ERROR_PAYLOAD.as_bytes());
    }
}
