//! 소비자가 주는 패킷 어휘 전부.
//!
//! ★이 trait 이 이 crate 의 존재 이유다★ — 지우면 `Out`/`In`/`Tag` 가 사라져 crate 가 바이트 파이프가
//! 된다(ADR-0177 이 기각한 후보 C). 그래서 여기만 dyn 이 아니라 **제네릭**이다.

use std::fmt::Display;
use std::hash::Hash;

use crate::frame::Frame;
use crate::stream::StreamMark;

/// 소비자의 패킷 어휘. ★crate 는 이 안의 어떤 이름도 모른다★ — 같은지(`Eq`)와 있는지(`Option`)만 본다.
///
/// ★역할 구분(요청/응답/알림/스트림)은 새 어휘가 아니라 아래 접근자들의 조합이다★. 나가는 쪽은
/// [`Wire::request_tag`] 의 유무로, 들어오는 쪽은 **[`Wire::reply_tag`] → [`Wire::stream_mark`] → 그 외**
/// 순으로 가른다. ★그 순서가 계약이다★ — 한 `In` 이 둘 다 `Some` 을 주면 **답장으로 먼저** 처리하고
/// [`crate::TransportEvent::AmbiguousRole`] 을 올린다(소비자 계약 위반 신호).
///
/// ★칸을 더하면 모든 소비자가 깨진다★ — 그래서 여섯이 지금 확정이다.
pub trait Wire: Send + Sync + 'static {
    /// 내가 보내는 것.
    type Out: Send + 'static;
    /// 내가 받는 것.
    type In: Send + 'static;
    /// 짝짓기 번호. ★바운드가 `Eq + Hash + Clone` 뿐인 것이 계약이다★ — 순서도 표현도 요구하지 않는다.
    type Tag: Eq + Hash + Clone + Send + Sync + 'static;
    /// 스트림 하나를 가리키는 키.
    type StreamKey: Eq + Hash + Clone + Send + Sync + 'static;
    /// 디코드 실패 사유. crate 는 사건에 문구로 실을 뿐 **분기하지 않는다**.
    type DecodeError: Display + Send + 'static;

    fn encode(&self, out: &Self::Out) -> Frame;

    /// [`Frame::Keepalive`] 는 여기 오지 않는다 — 살아있음 확인은 소비자 어휘와 만나지 않는다.
    fn decode(&self, frame: Frame) -> Result<Self::In, Self::DecodeError>;

    /// 나가는 것이 답장을 기다리나. `Some` 이면 보내기 **전에** 대기 슬롯을 만든다.
    fn request_tag(&self, out: &Self::Out) -> Option<Self::Tag>;

    /// 들어온 것이 누구의 답장인가. `Some` 이면 그 슬롯을 깨운다.
    fn reply_tag(&self, inn: &Self::In) -> Option<Self::Tag>;

    /// 들어온 것이 스트림 조각인가.
    fn stream_mark(&self, inn: &Self::In) -> Option<StreamMark<Self::StreamKey>>;

    /// 스트림을 이어받는 요청을 짓는다. ★crate 는 그 봉투를 모르고 "언제" 부를지만 안다★ — 구멍을
    /// 만났을 때와 소비자가 [`crate::Peer::resume_stream`] 을 불렀을 때다.
    ///
    /// `after` 는 **그 번호 뒤부터** 달라는 뜻이고, `None` 이면 상대가 가진 처음부터다.
    fn resume_request(&self, key: &Self::StreamKey, after: Option<u64>) -> Self::Out;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{TestIn, TestOut, TestWire};

    #[test]
    fn outbound_role_is_read_from_request_tag_alone() {
        let w = TestWire;
        assert_eq!(
            w.request_tag(&TestOut::Request {
                tag: 4,
                body: "x".into()
            }),
            Some(4)
        );
        assert_eq!(w.request_tag(&TestOut::Notice("x".into())), None);
        assert_eq!(
            w.request_tag(&TestOut::Resume {
                key: 1,
                after: None
            }),
            None
        );
    }

    #[test]
    fn inbound_roles_are_read_in_the_contracted_order() {
        let w = TestWire;
        let reply = TestIn::Reply {
            tag: 4,
            body: "ok".into(),
        };
        assert_eq!(w.reply_tag(&reply), Some(4));
        assert!(w.stream_mark(&reply).is_none());

        let chunk = TestIn::Chunk {
            key: 9,
            generation: 2,
            seq: 3,
            body: "b".into(),
        };
        assert!(w.reply_tag(&chunk).is_none());
        assert_eq!(
            w.stream_mark(&chunk).map(|m| (m.key, m.generation, m.seq)),
            Some((9, 2, 3))
        );

        let notice = TestIn::Notice("hi".into());
        assert!(w.reply_tag(&notice).is_none());
        assert!(w.stream_mark(&notice).is_none());
    }

    #[test]
    fn a_message_may_claim_both_roles_and_the_crate_can_see_it() {
        let w = TestWire;
        let both = TestIn::Ambiguous {
            tag: 1,
            key: 2,
            generation: 3,
            seq: 4,
        };
        assert!(w.reply_tag(&both).is_some());
        assert!(w.stream_mark(&both).is_some());
    }

    #[test]
    fn encode_decode_round_trips_both_frame_kinds() {
        let w = TestWire;
        let text = w.encode(&TestOut::Notice("hello".into()));
        assert!(matches!(text, Frame::Text(_)));
        let binary = w.encode(&TestOut::Blob(vec![1, 2, 3]));
        assert!(matches!(binary, Frame::Binary(_)));

        let decoded = w.decode(TestWire::encode_in(&TestIn::Notice("hello".into())));
        assert_eq!(decoded.unwrap(), TestIn::Notice("hello".into()));
    }

    #[test]
    fn decode_failure_is_a_display_only_value() {
        let w = TestWire;
        let err = w.decode(Frame::Text("nonsense".into())).unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn resume_request_is_built_by_the_consumer() {
        let w = TestWire;
        assert_eq!(
            w.resume_request(&7, Some(42)),
            TestOut::Resume {
                key: 7,
                after: Some(42)
            }
        );
    }
}
