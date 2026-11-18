//! Vocabulary of what flows over the link — frames and close. ★The crate does not interpret the **inside** of `Text`/`Binary`★ (only the consumer's [`crate::Wire`] does); it knows only where one chunk ends
//! and which code closed it.

use std::fmt;

/// One chunk the link carries. The two branches let one [`crate::Wire`] absorb a consumer with two codecs (ours: control = JSON text · output hot path = fixed-header binary).
/// ★`Debug` prints only the branch and byte length — not the body★ (TRD 3 §5 5): the handshake's first frame carries the auth token and logs · panic messages may print `{:?}`. Do not revert to `derive(Debug)`;
/// unpack the branch where the body is needed.
#[derive(Clone, PartialEq, Eq)]
pub enum Frame {
    Text(String),
    Binary(Vec<u8>),
    /// Liveness check. ★No payload, and it does not enter [`crate::Wire::decode`]★ — only arrival is the signal, so it never meets the consumer's vocabulary.
    Keepalive,
}

impl Frame {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CloseCode(pub u16);

impl CloseCode {
    pub const GOING_AWAY: Self = Self(4000);
    /// Rejected in the handshake — ★dialing again gives the same result★ ([`crate::RejectKind::Terminal`]): the dialer gives up without spending budget. The reason decision lives in the consumer's [`crate::Handshake`].
    pub const HANDSHAKE_REJECTED: Self = Self(4001);
    /// The peer went quiet (`idle_timeout`).
    pub const KEEPALIVE_TIMEOUT: Self = Self(4002);
    /// Passed the write deadline.
    pub const WRITE_DEADLINE: Self = Self(4003);
    /// Slow consumer — the queue is full.
    pub const QUEUE_FULL: Self = Self(4004);
    /// Decode broke.
    pub const PROTOCOL_ERROR: Self = Self(4005);
    /// Rejected in the handshake, but ★dialing again may connect★ ([`crate::RejectKind::Retryable`], e.g. a stale credential): the dialer spends reconnect budget and redials (TRD 3 §3-0 ⑤ · D20 ①).
    pub const HANDSHAKE_RETRYABLE: Self = Self(4006);
    /// Start of the consumer-only range. ★The crate uses only 4000~4099★ — a consumer using anything below collides.
    pub const CONSUMER_BASE: u16 = 4100;

    /// Whether this code falls in the range the crate reserved (4000~4099).
    pub fn is_reserved(self) -> bool {
        (4000..Self::CONSUMER_BASE).contains(&self.0)
    }
}

/// Close — a code and a human-readable message. ★Branch only on `code`★ — `reason` is for humans and changes without notice.
///
/// ★Few places let this code actually reach the peer (external fact)★ — RFC 6455 §7.4.1 reserves `1006` (never sent; §7.1.5: synthesized locally), and on a **congested socket** ([`CloseCode::QUEUE_FULL`] ·
/// [`CloseCode::WRITE_DEADLINE`]) there is no room to push a close frame. [`CloseCode::HANDSHAKE_REJECTED`] is not guaranteed either: the dialing side pushes the close only within the remaining handshake deadline, and
/// sends none if the last frame before the rejection could not get out (crate header 「A close code reaches the peer in few places」).
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
        let secret = r#"{"auth":"tok-SECRET-토큰"}"#; // hangul-ok
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
