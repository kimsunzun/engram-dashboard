//! 위로 올라가는 것의 어휘 — 자료형뿐이고 로직이 없다.
//!
//! ★올라가는 줄은 하나다★([`Incoming`]) — 패킷과 사건이 같은 줄로 흐른다. 옆 채널을 두지 않는 것이
//! 결정이다(ADR-0177 결정 9): 신호를 소비자가 **이미 읽는** 스트림에 넣지 않으면 안 들었을 때 없는 것과
//! 같다. 대가 = 패킷만 원하는 소비자도 `match` 를 한 겹 쓴다.
//!
//! ★이 명단이 곧 crate 경계다★(ADR-0182 「영향」) — **에이전트 오류·명령 실패는 여기 없다.** 위층이
//! 아는 일이고, 여러 출처를 합치는 것은 나중에 화면이 한다. 「이것도 같이 올리면 편하다」로 한 칸이라도
//! 들어오면 그 경계가 그 자리에서 깨진다.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use crate::frame::CloseCode;
use crate::machine::{ConnectFailure, DisconnectCause, GaveUpReason, Generation};
use crate::wire::Wire;

/// 상대에게 소비자가 붙이는 이름표. ★사건의 출처 표식이 이 값이고, 나중에 화면에 그대로 뜬다★.
///
/// crate 는 이 문자열을 해석하지 않는다 — 같은지만 본다.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PeerId(pub Arc<str>);

impl PeerId {
    pub fn new(name: impl AsRef<str>) -> Self {
        Self(Arc::from(name.as_ref()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 지터를 상대마다 흩는 씨앗. ★난수가 아니라 이름의 함수라 재실행에도 같다★(FNV-1a 64).
    pub fn jitter_seed(&self) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in self.0.as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }
}

impl From<&str> for PeerId {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl fmt::Display for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// 어느 방향에서 버려졌나.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// 위로 올라가는 줄이 찼다.
    Inbound,
    /// 상대에게 나가는 큐가 찼다.
    Outbound,
}

/// 상대에게서 온 패킷 하나.
pub struct Arrival<W: Wire> {
    /// ★출처 표식★ — 어느 상대의 것인가.
    pub peer: PeerId,
    /// ★세대 경계★ — 이 조각이 어느 **연결** 세대의 것인가. 재연결을 가로질러도 핸들은 살아 있으므로,
    /// 경계가 보이는 자리는 여기다.
    pub generation: Generation,
    pub msg: W::In,
}

impl<W: Wire> fmt::Debug for Arrival<W>
where
    W::In: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Arrival")
            .field("peer", &self.peer)
            .field("generation", &self.generation)
            .field("msg", &self.msg)
            .finish()
    }
}

/// 명부 위로 올라오는 것 전부.
pub enum Incoming<W: Wire> {
    Message(Arrival<W>),
    Event(TransportEvent<W>),
}

impl<W: Wire> Incoming<W> {
    pub fn peer(&self) -> &PeerId {
        match self {
            Incoming::Message(a) => &a.peer,
            Incoming::Event(e) => e.peer(),
        }
    }

    pub fn as_message(&self) -> Option<&Arrival<W>> {
        match self {
            Incoming::Message(a) => Some(a),
            Incoming::Event(_) => None,
        }
    }

    pub fn as_event(&self) -> Option<&TransportEvent<W>> {
        match self {
            Incoming::Event(e) => Some(e),
            Incoming::Message(_) => None,
        }
    }
}

impl<W: Wire> fmt::Debug for Incoming<W>
where
    W::In: fmt::Debug,
    W::Tag: fmt::Debug,
    W::StreamKey: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Incoming::Message(a) => f.debug_tuple("Message").field(a).finish(),
            Incoming::Event(e) => f.debug_tuple("Event").field(e).finish(),
        }
    }
}

/// 연결에 무슨 일이 있었나. ★전부 `peer` 를 갖는다★.
///
/// 스트림 계열의 `generation` 은 **화신 표식**(u32)이라 [`Generation`](연결 세대)과 다른 것이다 —
/// 이름이 같아 헷갈리기 쉬운 자리이므로 폭으로 갈린다고 기억하는 편이 빠르다.
/// ★`#[non_exhaustive]`★ — TRD §7 이 「항목은 늘어난다」로 못박았고 이 crate 는 재사용을 겨냥한다.
/// 지금 달아 두지 않으면 갈래 하나를 더하는 순간 소비자 전부의 `match` 가 깨진다.
#[non_exhaustive]
pub enum TransportEvent<W: Wire> {
    Connected {
        peer: PeerId,
        generation: Generation,
    },
    /// ★오늘 코드에는 이 `cause` 구분이 없다 — 전부 「끊김」 한 덩어리다★.
    Disconnected {
        peer: PeerId,
        generation: Generation,
        cause: DisconnectCause,
    },
    /// ★오늘은 재연결 중 실패 사유가 `debug!` 로만 남고 버려진다★.
    ConnectFailed {
        peer: PeerId,
        attempt: u32,
        of: u32,
        cause: ConnectFailure,
        retry_in: Option<Duration>,
    },
    /// 왜 거절당했나. ★이 문구가 팝업 문안이 된다★(ADR-0178 의 버전 거절이 여기로 올라온다).
    Rejected {
        peer: PeerId,
        code: Option<CloseCode>,
        reason: String,
    },
    /// 셸이 팝업 자격을 판정하는 입력(ADR-0180 결정 2).
    GaveUp { peer: PeerId, reason: GaveUpReason },
    /// 어디서 몇 개를 잃었고 무엇을 다시 청했나. ★오늘은 순번 구멍 검사가 아예 없다★.
    StreamGap {
        peer: PeerId,
        stream: W::StreamKey,
        generation: u32,
        expected: u64,
        got: u64,
        resumed_from: Option<u64>,
    },
    /// ★참/거짓이 아니라 **번호**★ — 지금 유효한 가장 오래된 순번(ADR-0177 결정 9).
    StreamTruncated {
        peer: PeerId,
        stream: W::StreamKey,
        generation: u32,
        oldest_valid: u64,
    },
    /// 화신이 갈렸다 — 순번 대조가 초기화됐다.
    StreamGenerationChanged {
        peer: PeerId,
        stream: W::StreamKey,
        from: u32,
        to: u32,
        /// 옛 화신 몫으로 붙들고 있다가 버린 조각 수. 새 화신에서는 그 번호들이 뜻을 갖지 않는다.
        discarded: u64,
    },
    /// 스트림 수 상한 때문에 이 스트림의 장부를 잊었다.
    ///
    /// ★상대가 새 키를 계속 지어내면 자리가 모자란다★ — 그때 무엇을 잊었는지 말하지 않으면, 이후
    /// 그 스트림의 중복 제거·구멍 검출이 조용히 처음부터 다시 시작된다. 붙들고 있던 조각은 버리지
    /// 않고 [`TransportEvent::StreamTruncated`] 와 함께 올라온다.
    StreamForgotten {
        peer: PeerId,
        stream: W::StreamKey,
        generation: u32,
        last_seq: u64,
    },
    RequestTimedOut {
        peer: PeerId,
        tag: W::Tag,
        after: Duration,
    },
    /// 답장이 왔는데 **기다리던 자리가 없다**.
    ///
    /// 만료 뒤 늦게 온 답장은 여기 오지 않는다 — 그건 설계대로 조용히 버린다(ADR-0181 결정 4). 여기
    /// 오는 것은 **애초에 우리가 청한 적 없는 짝짓기 번호**이고, 소비자 쪽 실수(답장을 받는 명령을
    /// `notify`/`broadcast` 로 밀었다)이거나 상대가 지어낸 번호다.
    UnmatchedReply { peer: PeerId, tag: W::Tag },
    /// 큐 정책이 [`crate::QueueFullPolicy::DropAndReport`] 일 때. ★조용한 유실을 안 만들기 위한 칸이다★.
    Dropped {
        peer: PeerId,
        direction: Direction,
        count: u64,
    },
    /// [`Wire::decode`] 가 실패했다. 연결을 끊을지는 [`crate::DecodeFailurePolicy`] 가 정한다.
    DecodeFailed {
        peer: PeerId,
        generation: Generation,
        reason: String,
    },
    /// 한 `In` 이 답장이면서 스트림 조각이라고 신고됐다 — [`Wire`] 계약 위반 신호다.
    AmbiguousRole { peer: PeerId },
}

impl<W: Wire> TransportEvent<W> {
    pub fn peer(&self) -> &PeerId {
        match self {
            Self::Connected { peer, .. }
            | Self::Disconnected { peer, .. }
            | Self::ConnectFailed { peer, .. }
            | Self::Rejected { peer, .. }
            | Self::GaveUp { peer, .. }
            | Self::StreamGap { peer, .. }
            | Self::StreamTruncated { peer, .. }
            | Self::StreamGenerationChanged { peer, .. }
            | Self::StreamForgotten { peer, .. }
            | Self::RequestTimedOut { peer, .. }
            | Self::UnmatchedReply { peer, .. }
            | Self::Dropped { peer, .. }
            | Self::DecodeFailed { peer, .. }
            | Self::AmbiguousRole { peer } => peer,
        }
    }

    /// 로그·단언에서 갈래를 집기 위한 안정된 이름.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Connected { .. } => "Connected",
            Self::Disconnected { .. } => "Disconnected",
            Self::ConnectFailed { .. } => "ConnectFailed",
            Self::Rejected { .. } => "Rejected",
            Self::GaveUp { .. } => "GaveUp",
            Self::StreamGap { .. } => "StreamGap",
            Self::StreamTruncated { .. } => "StreamTruncated",
            Self::StreamGenerationChanged { .. } => "StreamGenerationChanged",
            Self::StreamForgotten { .. } => "StreamForgotten",
            Self::RequestTimedOut { .. } => "RequestTimedOut",
            Self::UnmatchedReply { .. } => "UnmatchedReply",
            Self::Dropped { .. } => "Dropped",
            Self::DecodeFailed { .. } => "DecodeFailed",
            Self::AmbiguousRole { .. } => "AmbiguousRole",
        }
    }
}

impl<W: Wire> fmt::Debug for TransportEvent<W>
where
    W::Tag: fmt::Debug,
    W::StreamKey: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connected { peer, generation } => f
                .debug_struct("Connected")
                .field("peer", peer)
                .field("generation", generation)
                .finish(),
            Self::Disconnected {
                peer,
                generation,
                cause,
            } => f
                .debug_struct("Disconnected")
                .field("peer", peer)
                .field("generation", generation)
                .field("cause", cause)
                .finish(),
            Self::ConnectFailed {
                peer,
                attempt,
                of,
                cause,
                retry_in,
            } => f
                .debug_struct("ConnectFailed")
                .field("peer", peer)
                .field("attempt", attempt)
                .field("of", of)
                .field("cause", cause)
                .field("retry_in", retry_in)
                .finish(),
            Self::Rejected { peer, code, reason } => f
                .debug_struct("Rejected")
                .field("peer", peer)
                .field("code", code)
                .field("reason", reason)
                .finish(),
            Self::GaveUp { peer, reason } => f
                .debug_struct("GaveUp")
                .field("peer", peer)
                .field("reason", reason)
                .finish(),
            Self::StreamGap {
                peer,
                stream,
                generation,
                expected,
                got,
                resumed_from,
            } => f
                .debug_struct("StreamGap")
                .field("peer", peer)
                .field("stream", stream)
                .field("generation", generation)
                .field("expected", expected)
                .field("got", got)
                .field("resumed_from", resumed_from)
                .finish(),
            Self::StreamTruncated {
                peer,
                stream,
                generation,
                oldest_valid,
            } => f
                .debug_struct("StreamTruncated")
                .field("peer", peer)
                .field("stream", stream)
                .field("generation", generation)
                .field("oldest_valid", oldest_valid)
                .finish(),
            Self::StreamGenerationChanged {
                peer,
                stream,
                from,
                to,
                discarded,
            } => f
                .debug_struct("StreamGenerationChanged")
                .field("peer", peer)
                .field("stream", stream)
                .field("from", from)
                .field("to", to)
                .field("discarded", discarded)
                .finish(),
            Self::StreamForgotten {
                peer,
                stream,
                generation,
                last_seq,
            } => f
                .debug_struct("StreamForgotten")
                .field("peer", peer)
                .field("stream", stream)
                .field("generation", generation)
                .field("last_seq", last_seq)
                .finish(),
            Self::RequestTimedOut { peer, tag, after } => f
                .debug_struct("RequestTimedOut")
                .field("peer", peer)
                .field("tag", tag)
                .field("after", after)
                .finish(),
            Self::UnmatchedReply { peer, tag } => f
                .debug_struct("UnmatchedReply")
                .field("peer", peer)
                .field("tag", tag)
                .finish(),
            Self::Dropped {
                peer,
                direction,
                count,
            } => f
                .debug_struct("Dropped")
                .field("peer", peer)
                .field("direction", direction)
                .field("count", count)
                .finish(),
            Self::DecodeFailed {
                peer,
                generation,
                reason,
            } => f
                .debug_struct("DecodeFailed")
                .field("peer", peer)
                .field("generation", generation)
                .field("reason", reason)
                .finish(),
            Self::AmbiguousRole { peer } => {
                f.debug_struct("AmbiguousRole").field("peer", peer).finish()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestWire;

    #[test]
    fn peer_id_is_an_opaque_label() {
        let a = PeerId::new("local-daemon");
        assert_eq!(a.as_str(), "local-daemon");
        assert_eq!(a.to_string(), "local-daemon");
        assert_eq!(a, PeerId::from("local-daemon"));
        assert_ne!(a, PeerId::new("remote"));
    }

    #[test]
    fn jitter_seed_is_stable_per_name_and_differs_across_names() {
        assert_eq!(
            PeerId::new("a").jitter_seed(),
            PeerId::new("a").jitter_seed()
        );
        assert_ne!(
            PeerId::new("a").jitter_seed(),
            PeerId::new("b").jitter_seed()
        );
    }

    #[test]
    fn every_event_carries_its_source() {
        let peer = PeerId::new("d1");
        let events: Vec<TransportEvent<TestWire>> = vec![
            TransportEvent::Connected {
                peer: peer.clone(),
                generation: Generation(1),
            },
            TransportEvent::AmbiguousRole { peer: peer.clone() },
            TransportEvent::Dropped {
                peer: peer.clone(),
                direction: Direction::Outbound,
                count: 3,
            },
            TransportEvent::StreamTruncated {
                peer: peer.clone(),
                stream: 1,
                generation: 2,
                oldest_valid: 40,
            },
        ];
        for e in &events {
            assert_eq!(e.peer(), &peer, "{}", e.kind());
        }
    }

    #[test]
    fn the_event_list_is_open_for_growth() {
        // crate 밖에서 이 enum 을 남김없이 match 할 수 없다는 것이 `#[non_exhaustive]` 의 전부다.
        // 여기서는 갈래 이름이 안정된 문자열로 나오는 것만 잰다 — 소비자가 그것으로 가른다.
        let peer = PeerId::new("d1");
        let forgotten: TransportEvent<TestWire> = TransportEvent::StreamForgotten {
            peer: peer.clone(),
            stream: 3,
            generation: 1,
            last_seq: 9,
        };
        assert_eq!(forgotten.kind(), "StreamForgotten");
        assert_eq!(forgotten.peer(), &peer);
    }

    #[test]
    fn incoming_reads_the_source_from_either_arm() {
        let peer = PeerId::new("d1");
        let msg: Incoming<TestWire> = Incoming::Message(Arrival {
            peer: peer.clone(),
            generation: Generation(2),
            msg: crate::testing::TestIn::Notice("x".into()),
        });
        assert_eq!(msg.peer(), &peer);
        assert!(msg.as_message().is_some());
        assert!(msg.as_event().is_none());

        let ev: Incoming<TestWire> = Incoming::Event(TransportEvent::AmbiguousRole { peer });
        assert!(ev.as_event().is_some());
        assert!(ev.as_message().is_none());
    }
}
