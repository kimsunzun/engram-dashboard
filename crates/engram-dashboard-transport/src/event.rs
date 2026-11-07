//! 위로 올라가는 것의 어휘 — 자료형뿐이고 로직이 없다.
//!
//! ★손잡이마다 올라가는 줄은 하나다★([`Incoming`]) — 패킷과 사건이 같은 줄로 흐른다. 옆 채널을 두지 않는 것이
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
///
/// ★받는 쪽 갈래는 없다★ — 위로 올라가는 줄이 차도 감독이 자기 순서 칸에 붙들어 버리지 않는다
/// (`peer.rs` 헤더 「순서 칸」 · TRD 3 §3-0 ②).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// 상대에게 나갈 것을 못 내보냈다. 큐 포화만이 아니라 끊겨서 버린 것도 들고, 그중에는 **쓰는 도중
    /// 끊긴** 알림·팬아웃도 있다(같은 처지의 요청은 여기 안 들고 오류로 깨어난다).
    ///
    /// ★「확실히 잃었다」가 아니라 「쓰기가 `Ok` 로 끝나는 것을 못 봤다」다 — 부풀 수 있다★. 부푸는 자리는
    /// 쓰는 도중 끊긴 프레임이다 — 이미 어댑터 버퍼에 들어가 있었을 수 있고, WS 어댑터에서는 통로를 버릴 때
    /// 보내는 닫기가 그것을 먼저 밀어낼 수 있다(소스 판독 — `ws.rs` 「통로 성질」). 같은 혼잡이면 그 닫기도
    /// 쓰기 시한에 잘릴 수 있다.
    ///
    /// [`crate::Registry::broadcast`] 가 못 넣은 프레임은 그 상대의 감독이 한 번 센다(큐 포화 · 운영 단계
    /// 아님). 단 앱이 [`crate::Peer::close`] 를 부른 뒤 운영 단계가 아니라서 못 넣은 것은 세지 않는다(`lib.rs`
    /// 「알려진 한계」) — 닫기를 부른 뒤라도 아직 운영 중인 상대에 넣다 큐가 찬 것은 센다.
    ///
    /// ★묶인 세대가 쓰기 직전에 지금 세대가 아니라서 안 쓴 것도 든다★(TRD 3 §3-0 ③ — [`crate::Peer::notify_in`])
    /// — 큐에 넣을 때는 그 연결이 운영 중이었지만 쓰기 전에 다시 붙었다. 그 신고는 다음 연결의
    /// [`TransportEvent::Connected`] 뒤에 올 수 있다 — 버린 것은 앞 연결 몫이다. `notify` · `notify_in` 이 입구에서
    /// 묶은 세대가 운영 중이 아니라 `Err`([`crate::SendError::Disconnected`])를 받은 것은 세지 않는다 — 큐가 차 받은
    /// `Err` 는 센다(큐 포화). 팬아웃 · 이어받기는 부르는 쪽이 오류를 못 받으므로 입구의 거절도 센다(위). 끝난
    /// 세대에서 찬 큐는 신고만 하고 다음 연결을 끊지 않는다.
    ///
    /// 거꾸로 안 센 것도 도착을 보증하지 않는다 — 쓰기의 `Ok` 는 전송 버퍼까지다.
    Outbound,
}

/// 상대에게서 온 패킷 하나.
pub struct Arrival<W: Wire> {
    /// ★출처 표식★ — 어느 상대의 것인가.
    pub peer: PeerId,
    /// ★세대 경계★ — 이 조각이 어느 **연결** 세대의 것인가. 재연결을 가로질러도 핸들은 살아 있으므로,
    /// 경계가 보이는 자리는 여기다.
    ///
    /// ★스트림 조각은 더 새 세대의 [`TransportEvent::Connected`] 뒤에 옛 세대 번호를 달고 올 수 있다★ — 이어받기
    /// 창이 붙든 조각은 다시 붙은 뒤 닫힐 때 올라가도 실어 온 연결의 세대를 단다. 그래서 스트림 층을 켜는 결정이
    /// 서면(지금 두 소비자는 안 쓴다 — ADR-0310 결정 16) 그 받는 쪽은 스트림 조각을 세대로 거르지 않아야 한다 —
    /// 거르면 창이 붙들었다 올린 조각을 버린다. 세대 거름은 스트림이 아닌 메시지에만 건다.
    ///
    /// ★이 메시지에 걸린 답 · 요청을 실어 온 그 연결로만 내려면 이 값을 [`crate::Peer::notify_in`] ·
    /// [`crate::Peer::request_in`] 에 넘긴다★ — 그 사이 다시 붙었으면 다음 연결로 넘어가지 않고 안 나간다(TRD 3
    /// §3-0 ③).
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

/// 수신단([`crate::Inbox`])으로 올라오는 것 전부.
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
    /// ★종결 거절([`crate::RejectKind::Terminal`])★ — 바로 뒤에 [`TransportEvent::GaveUp`] 이 온다(ADR-0178 의 버전
    /// 거절 · 거는 쪽의 [`crate::DialFailure::Refused`] 가 여기로 올라온다). 다시 거는 거절은 여기 오지 않고
    /// [`TransportEvent::ConnectFailed`] 의 `cause` 로 간다.
    ///
    /// ★팝업 문안은 `code` 로 고르는 고정 로컬 문구다 — `reason` 이 아니다★(TRD 3 §3-0 ⑤). `reason` 은 상대가 준 글일
    /// 수 있어 가르지도 화면 문안으로 쓰지도 않고 세부 · 로그로만 보낸다 — 가린 글이다
    /// ([`crate::mask_reason`] · [`crate::ConnectFailure::Rejected`]).
    Rejected {
        peer: PeerId,
        code: CloseCode,
        reason: String,
    },
    /// 셸이 팝업 자격을 판정하는 입력(ADR-0180 결정 2).
    GaveUp { peer: PeerId, reason: GaveUpReason },
    /// ★이 감독의 마지막 사건이자 그 손잡이 수신단([`crate::Inbox`])의 마지막 항목★ — 정리가 통로를 다 닫고
    /// 순서 칸에 붙든 것을 다 실은 뒤 맨 끝에 내고, 감독이 넣는 끝을 놓아 그 수신단의 다음 `recv` 는 `None`
    /// 이다(TRD 3 §3-0 ②).
    ///
    /// `generation` = 이 감독의 마지막 연결 세대(한 번도 안 붙었으면 명부가 준 바닥값) · `cause` = 끝낸 까닭.
    /// ★건 상대([`crate::Registry::add`])는 늘 [`DisconnectCause::Explicit`] 다★ — 감독이 끝나는 길이 닫기 하나뿐이다
    /// (앱이 닫았거나, 그 손잡이가 다 사라졌거나, 받는 쪽이 수신단을 닫았다). ★받은 상대([`crate::Registry::attach`])는
    /// 진짜 까닭이다★ — 다시 걸지 않아 연결을 잃는 것이 곧 끝이다: 운영 중 끊김이면 앞의 `Disconnected` 와 같은 까닭,
    /// 운영 단계에 못 들었으면 핸드셰이크가 멈춘 자리(우리 핸드셰이크의 거절 = `Explicit` · 상대가 닫음 = `PeerClosed` ·
    /// 읽기 · 쓰기 오류 = `ReadFailed` · `WriteFailed` · 쓰기가 시한을 넘김 = `WriteDeadline` · 인증 시한까지 상대가 못
    /// 마침 = `Idle`), 앱 · 받는 쪽이 닫았으면 `Explicit`.
    ///
    /// ★`generation` 으로 감독을 가르지 않는다 — 수신단이 가른다★ — 바닥값은 같은 이름표의 앞 감독이
    /// 마지막으로 뽑은 세대라, 한 번도 안 붙고 끝난 감독의 이것은 앞 감독의 이것과 똑같을 수 있다. 감독마다
    /// 수신단이 따로라 둘이 한 수신단에 섞이지 않는다.
    ///
    /// ★안 올 수 있다★ — 받는 쪽이 수신단을 닫거나 놓은 뒤에는 실을 데가 없다(닫기 전에 이미 실린 이것은
    /// 닫은 뒤에도 꺼낼 수 있다). 감독이 정상 정리 없이 끝나도
    /// 안 온다 — 산 채 안 비우는 받는 쪽에 정리가 상한(`peer.rs` 의 `CLOSE_DRAIN_LIMIT`)을 넘겨 남은 것과 함께
    /// 이것도 버렸거나(세지도 신고하지도 않는다), 태스크가 비정상으로 끝났다(패닉 · 런타임 종료). 어느 쪽이든 넣는
    /// 끝이 떨어져 `Closed` 없는 `None` 이 끝을 알린다 — 그것을 상한 초과로만 읽지 말 것.
    ///
    /// 운영 중 끊김의 읽기 끝 · 쓰기 끝 가름([`DisconnectCause::ReadFailed`] · [`DisconnectCause::WriteFailed`])은
    /// 그 앞의 [`TransportEvent::Disconnected`] 가 싣는다 — 받은 상대는 이것도 싣는다(데몬 어댑터가 이 까닭 하나로 읽기
    /// 끝 · 쓰기 끝을 가른다 — TRD 3 §3-0 ①).
    Closed {
        peer: PeerId,
        generation: Generation,
        cause: DisconnectCause,
    },
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
    /// 버린 것의 수 — 무엇을 세나는 [`Direction`] 각 갈래가 정본이다. 큐 정책과 무관하게 올라온다.
    /// ★조용한 유실을 안 만들기 위한 칸이다★.
    Dropped {
        peer: PeerId,
        direction: Direction,
        count: u64,
    },
    /// [`Wire::decode`] 가 실패했다. 연결을 끊을지는 [`crate::DecodeFailurePolicy`] 가 정한다.
    ///
    /// `reason` = 소비자 해독 오류의 분류([`crate::wire::DecodeClass::class`]) — ★고정 낱말이고 받은 값을 안 담는다★.
    DecodeFailed {
        peer: PeerId,
        generation: Generation,
        reason: &'static str,
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
            | Self::Closed { peer, .. }
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
            Self::Closed { .. } => "Closed",
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
            Self::Closed {
                peer,
                generation,
                cause,
            } => f
                .debug_struct("Closed")
                .field("peer", peer)
                .field("generation", generation)
                .field("cause", cause)
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
            TransportEvent::Closed {
                peer: peer.clone(),
                generation: Generation(1),
                cause: DisconnectCause::Explicit,
            },
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
