//! 출력 프레임 자리채움 — 싣지 못한 사건의 seq 를 비우지 않는다(ADR-0231).
//!
//! ★왜 있나(load-bearing)★: 뷰는 live 에서 `seq > 마지막+1` 을 시한 없이 붙들고 `마지막+1` 이 와야 흘린다.
//! 그래서 데몬 sink 나 셸 중계가 한 seq 를 말없이 버리면 그 뒤 프레임이 붙듦 상한까지 쌓이고 — 한가한
//! 에이전트면 영영 — 그 뷰가 멈춘다. 버리는 자리는 대신 이 프레임을 **같은 seq 로** 보낸다: tag1
//! `StructuredEvent::Error` 에 고정 문구. 챗 뷰는 오류 행 하나를 그리고 터미널 뷰는 tag1 을 무시하되 seq 는
//! 전진한다.
//!
//! ★한 벌을 데몬 sink 와 셸이 같이 쓴다★ — 두 자리의 자리채움이 바이트까지 같아야 뷰가 출처를 가리지
//! 않는다(시험이 그 동일성을 잰다).
//! ★문구는 고정 문자열이고 페이로드는 컴파일 때 굳는다★ — 자리채움이 버려진 사건을 대신하는 것이므로
//! 그 자신의 직렬화가 실패할 여지가 있으면 안 된다. 그래서 런타임 직렬화 대신 상수를 싣고, 그 상수가
//! `StructuredEvent::Error` 의 직렬화와 같음을 시험이 지킨다.

use crate::codec::encode_structured_frame;
use crate::ids::AgentId;

// 문구 리터럴 한 곳 — 공개 상수와 페이로드 상수가 둘 다 여기서 펴진다(`concat!` 은 리터럴만 받는다).
macro_rules! placeholder_message {
    () => {
        "이 출력 사건을 싣지 못했습니다"
    };
}

/// 자리채움 `Error` 의 문구. 사람이 읽는 불투명 문자열이다 — 소비자가 이 값으로 분기하지 않는다.
pub const PLACEHOLDER_ERROR_MESSAGE: &str = placeholder_message!();

/// `StructuredEvent::Error { message: PLACEHOLDER_ERROR_MESSAGE }` 의 JSON 직렬화 그대로.
const PLACEHOLDER_ERROR_PAYLOAD: &str = concat!(
    r#"{"type":"Error","message":""#,
    placeholder_message!(),
    r#""}"#
);

/// `agent_id`·`epoch`·`seq` 를 단 tag1 자리채움 프레임.
// ADR-0231
pub fn placeholder_error_frame(agent_id: AgentId, epoch: u32, seq: u64) -> Vec<u8> {
    encode_structured_frame(agent_id, epoch, seq, PLACEHOLDER_ERROR_PAYLOAD.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{decode_frame, FRAME_TAG_STRUCTURED_EVENT};
    use crate::messages::StructuredEvent;

    // 상수 페이로드가 손으로 쓴 JSON 이라 wire 타입과 어긋날 수 있다 — 그 어긋남을 두 방향으로 잡는다.
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
