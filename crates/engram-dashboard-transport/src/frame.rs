//! Vocabulary of what flows over the link — frames and close.
//!
//! ★This crate does not interpret the **inside** of `Text`/`Binary`★. Interpretation belongs only to the consumer's [`crate::Wire`], and
//! what is here is only "where one chunk ends" and "which code it was closed with".

use std::fmt;

/// One chunk the link carries.
///
/// It is thanks to these two branches that one [`crate::Wire`] can absorb a consumer with two codecs (our wire is
/// control = JSON text · output hot path = fixed-header binary).
///
/// ★`Debug` prints only the branch and the byte length — not the body★ (TRD 3 §5 5). The handshake's first frame carries the auth token,
/// and a consumer's log · panic message may print a frame with `{:?}`. Do not revert to `derive(Debug)` — where the body is needed,
/// unpack the branch and take it out directly.
#[derive(Clone, PartialEq, Eq)]
pub enum Frame {
    Text(String),
    Binary(Vec<u8>),
    /// Liveness check. ★It has no payload and does not enter [`crate::Wire::decode`]★ — only the fact of arrival is
    /// the signal, so it never meets the consumer's vocabulary.
    Keepalive,
}

impl Frame {
    /// Unit of the received-byte tally (`budget.rs`) — the payload length. A liveness check is 0.
    pub(crate) fn payload_len(&self) -> usize {
        match self {
            Frame::Text(text) => text.len(),
            Frame::Binary(bytes) => bytes.len(),
            Frame::Keepalive => 0,
        }
    }
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Frame::Text(text) => f.debug_struct("Text").field("bytes", &text.len()).finish(),
            Frame::Binary(bytes) => f
                .debug_struct("Binary")
                .field("bytes", &bytes.len())
                .finish(),
            Frame::Keepalive => f.write_str("Keepalive"),
        }
    }
}

/// Close code. 4000~4999 = the application range of RFC 6455 §7.4.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CloseCode(pub u16);

impl CloseCode {
    /// Our side exits normally.
    pub const GOING_AWAY: Self = Self(4000);
    /// Rejected in the handshake — ★dialing again gives the same result★ ([`crate::RejectKind::Terminal`]). On receiving this code the dialer
    /// gives up without spending any budget. The decision on the reason lives inside the consumer's [`crate::Handshake`].
    pub const HANDSHAKE_REJECTED: Self = Self(4001);
    /// The peer went quiet (`idle_timeout`).
    pub const KEEPALIVE_TIMEOUT: Self = Self(4002);
    /// Passed the write deadline.
    pub const WRITE_DEADLINE: Self = Self(4003);
    /// Slow consumer — the queue is full.
    pub const QUEUE_FULL: Self = Self(4004);
    /// Decode broke.
    pub const PROTOCOL_ERROR: Self = Self(4005);
    /// Rejected in the handshake, but ★dialing again may connect★ ([`crate::RejectKind::Retryable`] — e.g. a stale credential). On receiving
    /// this code the dialer spends reconnect budget and dials again (TRD 3 §3-0 ⑤ · D20 ①).
    pub const HANDSHAKE_RETRYABLE: Self = Self(4006);
    /// Start of the consumer-only range. ★The crate uses only 4000~4099★ — if a consumer uses anything below that, they collide.
    pub const CONSUMER_BASE: u16 = 4100;

    /// Whether this code falls in the range the crate reserved (4000~4099).
    pub fn is_reserved(self) -> bool {
        (4000..Self::CONSUMER_BASE).contains(&self.0)
    }
}

/// Close — carries the code and a human-readable message together.
///
/// ★Branch only on `code`★ — `reason` is for humans to read and changes without notice (the comment on today's wire
/// `SubscribeFailed { reason }` already states the same contract).
///
/// ★The place where this code actually reaches the peer is narrow (external fact)★ — RFC 6455 §7.4.1 pins `1006` down as a
/// **reserved value that cannot be sent on the wire**, and §7.1.5 defines it as a value the receiving side synthesizes locally. So when
/// disconnecting on a **congested socket**, as with [`CloseCode::QUEUE_FULL`] · [`CloseCode::WRITE_DEADLINE`], there is no room to push a close
/// frame in and the peer cannot see the code. [`CloseCode::HANDSHAKE_REJECTED`] is not guaranteed either — when the dialing side's handshake
/// rejects, it pushes the close in only within the remaining handshake deadline, and if the last frame before the rejection could not get out, no close is
/// sent at all (details = crate header
/// 「닫기 코드가 상대에게 닿는 자리는 좁다」 (the place where close codes reach the peer is narrow)).
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
            CloseCode::HANDSHAKE_RETRYABLE,
        ] {
            assert!(code.0 < CloseCode::CONSUMER_BASE, "{code:?}");
        }
    }

    #[test]
    fn debug_shows_the_arm_and_byte_length_but_never_the_body() {
        let secret = r#"{"auth":"tok-SECRET-토큰"}"#;
        let cases = [
            (
                Frame::Text(secret.into()),
                format!("Text {{ bytes: {} }}", secret.len()),
            ),
            (
                Frame::Binary(secret.as_bytes().to_vec()),
                format!("Binary {{ bytes: {} }}", secret.len()),
            ),
            (Frame::Keepalive, "Keepalive".to_string()),
        ];
        for (frame, expected) in cases {
            for shown in [format!("{frame:?}"), format!("{frame:#?}")] {
                assert!(!shown.contains("SECRET"), "{shown}");
                // First byte of the byte body, `{` = 123 — it must not leak even when printed as a number list.
                assert!(!shown.contains("123"), "{shown}");
            }
            assert_eq!(format!("{frame:?}"), expected);
        }
    }

    #[test]
    fn keepalive_carries_nothing() {
        assert_eq!(Frame::Keepalive, Frame::Keepalive);
        assert_ne!(Frame::Keepalive, Frame::Text(String::new()));
        assert_ne!(Frame::Text(String::new()), Frame::Binary(Vec::new()));
    }
}
