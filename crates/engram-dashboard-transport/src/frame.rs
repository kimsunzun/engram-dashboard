//! 통로 위로 흐르는 것의 어휘 — 프레임과 닫기.
//!
//! ★이 crate 는 `Text`/`Binary` 의 **안을 해석하지 않는다**★. 해석은 소비자의 [`crate::Wire`] 뿐이고,
//! 여기 있는 것은 "한 덩어리가 어디서 끝나나" 와 "어떤 코드로 닫았나" 뿐이다.

/// 통로가 나르는 한 덩어리.
///
/// 코덱이 둘인 소비자를 한 [`crate::Wire`] 가 흡수할 수 있는 것이 이 두 갈래 덕이다(우리 wire 는
/// control = JSON text · 출력 hot path = 고정헤더 binary 다).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    Text(String),
    Binary(Vec<u8>),
    /// 살아있음 확인. ★payload 가 없고 [`crate::Wire::decode`] 에 들어가지 않는다★ — 도착 사실만이
    /// 신호라 소비자 어휘와 만나지 않는다.
    Keepalive,
}

/// 닫기 코드. 4000~4999 = RFC 6455 §7.4.2 의 애플리케이션 구간이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CloseCode(pub u16);

impl CloseCode {
    /// 내 쪽이 정상 종료한다.
    pub const GOING_AWAY: Self = Self(4000);
    /// 핸드셰이크에서 거절했다. 사유 판정은 소비자의 [`crate::Handshake`] 안에 있다.
    pub const HANDSHAKE_REJECTED: Self = Self(4001);
    /// 상대가 조용해졌다(`idle_timeout`).
    pub const KEEPALIVE_TIMEOUT: Self = Self(4002);
    /// 쓰기 시한을 넘겼다.
    pub const WRITE_DEADLINE: Self = Self(4003);
    /// 느린 소비자 — 큐가 찼다.
    pub const QUEUE_FULL: Self = Self(4004);
    /// 디코드가 깨졌다.
    pub const PROTOCOL_ERROR: Self = Self(4005);
    /// 소비자 전용 구간의 시작. ★crate 는 4000~4099 만 쓴다★ — 그 아래를 소비자가 쓰면 충돌한다.
    pub const CONSUMER_BASE: u16 = 4100;

    /// 이 코드가 crate 가 예약한 구간(4000~4099)에 드나.
    pub fn is_reserved(self) -> bool {
        (4000..Self::CONSUMER_BASE).contains(&self.0)
    }
}

/// 닫기 — 코드와 사람이 읽는 문구를 함께 싣는다.
///
/// ★분기는 `code` 로만 한다★ — `reason` 은 사람이 읽는 것이고 예고 없이 바뀐다(오늘 wire 의
/// `SubscribeFailed { reason }` 주석이 이미 같은 계약을 적어 두었다).
///
/// ★이 코드가 상대에게 실제로 닿는 자리는 좁다(외부 사실)★ — RFC 6455 §7.4.1 이 `1006` 을 **와이어로
/// 보낼 수 없는 예약값**으로 못박고 §7.1.5 가 그것을 수신 측이 로컬 합성하는 값으로 정의한다. 즉
/// [`CloseCode::QUEUE_FULL`]·[`CloseCode::WRITE_DEADLINE`] 처럼 **혼잡한 소켓**에서 끊을 때는 닫기
/// 프레임을 밀어 넣을 자리가 없어 상대는 코드를 못 본다. 실제로 전달되는 자리는
/// [`CloseCode::HANDSHAKE_REJECTED`] 다(그때는 아무것도 안 밀려 있다).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Close {
    pub code: CloseCode,
    pub reason: String,
}

impl Close {
    pub fn new(code: CloseCode, reason: impl Into<String>) -> Self {
        Self {
            code,
            reason: reason.into(),
        }
    }

    pub fn going_away() -> Self {
        Self::new(CloseCode::GOING_AWAY, "going away")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_range_is_4000_to_4099() {
        assert!(CloseCode::GOING_AWAY.is_reserved());
        assert!(CloseCode::PROTOCOL_ERROR.is_reserved());
        assert!(!CloseCode(CloseCode::CONSUMER_BASE).is_reserved());
        assert!(!CloseCode(1006).is_reserved());
    }

    #[test]
    fn crate_codes_stay_below_consumer_base() {
        for code in [
            CloseCode::GOING_AWAY,
            CloseCode::HANDSHAKE_REJECTED,
            CloseCode::KEEPALIVE_TIMEOUT,
            CloseCode::WRITE_DEADLINE,
            CloseCode::QUEUE_FULL,
            CloseCode::PROTOCOL_ERROR,
        ] {
            assert!(code.0 < CloseCode::CONSUMER_BASE, "{code:?}");
        }
    }

    #[test]
    fn keepalive_carries_nothing() {
        assert_eq!(Frame::Keepalive, Frame::Keepalive);
        assert_ne!(Frame::Keepalive, Frame::Text(String::new()));
        assert_ne!(Frame::Text(String::new()), Frame::Binary(Vec::new()));
    }
}
