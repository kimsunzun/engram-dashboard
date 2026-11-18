//! Output frame placeholder — fills the seq of an event that could not be carried (ADR-0231).
//!
//! ★Why it exists★: a live view holds `seq > last+1` with no deadline until `last+1` arrives, so a silently dropped seq stalls the view
//!   (forever for an
//! idle agent). The dropping spot sends this frame **with the same seq**: a tag1 `StructuredEvent::Error` with fixed wording. The chat
//!   view draws one error row;
//! the terminal view ignores tag1 but the seq advances.
//!
//! ★The daemon sink and the shell share one set★ — both must emit identical bytes so the view cannot tell the source (a test measures it).
//! ★The payload is a compile-time constant, not runtime serialization★: the placeholder stands for a dropped event, so it must have no
//!   room to fail serializing;
//! a test guards that the constant equals the serialization of `StructuredEvent::Error`.

use crate::codec::encode_structured_frame;
use crate::ids::AgentId;

// The single place for the wording literal — the public constant and the payload constant are both expanded from here (`concat!` accepts
//   only literals).
macro_rules! placeholder_message {
    () => {
        "This output event could not be delivered"
    };
}

/// Wording of the placeholder `Error` — a fixed marker. The chat view compares an error item's message with it (byte-equal mirror in
/// `src/api/wsFrame.ts`) and shows its own translated text instead; no other consumer branches on it.
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
            serde_json::to_vec(&event).expect("serialize").as_slice(),
            "constant payload = serialization of StructuredEvent::Error"
        );
        let parsed: StructuredEvent =
            serde_json::from_str(PLACEHOLDER_ERROR_PAYLOAD).expect("reads as the wire type");
        assert_eq!(parsed, event);
    }

    #[test]
    fn the_frame_carries_the_given_header_as_tag1() {
        let agent = AgentId::from_u128(0x0102_0304_0506_0708_090a_0b0c_0d0e_0f10);
        let frame = placeholder_error_frame(agent, 7, 42);
        let decoded = decode_frame(&frame).expect("valid tag1 frame");
        assert_eq!(decoded.tag, FRAME_TAG_STRUCTURED_EVENT);
        assert_eq!(decoded.agent_id, agent);
        assert_eq!(decoded.epoch, 7);
        assert_eq!(
            decoded.seq, 42,
            "carries the seq of the dropped event as is"
        );
        assert_eq!(decoded.payload, PLACEHOLDER_ERROR_PAYLOAD.as_bytes());
    }
}
