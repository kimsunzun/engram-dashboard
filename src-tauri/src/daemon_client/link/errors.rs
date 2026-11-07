//! Transport send and request failures as the texts the shell gives its callers (TRD S21 trd-3 §6-2 U2).
//!
//! ★The texts are a contract★ — callers tell outcomes apart by these strings (the frontend, the LLM surface), so they
//! stay byte-identical to what the shell gave before it ran on the transport. Failures the shell never had before (a
//! full queue, a retired request id) read as its send failure with the transport's reason.
//!
//! ★Pre-check first★ — [`RequestError::Disconnected`] with `sent: false` covers both "the peer was not live when
//! called" and "it dropped before the command reached the wire". Those two get different texts ([`NOT_CONNECTED`] ·
//! [`UNSENT_ON_DISCONNECT`]), so a caller runs [`require_live`] before `request` / `notify` and maps what the transport
//! returns afterwards with [`request_error_text`] / [`send_error_text`].
// ADR-0195

use std::time::Duration;

use engram_dashboard_transport::{PeerState, RequestError, SendError};

use crate::daemon_client::{CHANNEL_GONE, NOT_CONNECTED, NO_REQUEST_ID};

/// The text of a command that never reached the wire because the link dropped. "Safe to resend" holds on this path
/// only — one that went out has an unknown outcome ([`SENT_OUTCOME_UNKNOWN`]).
pub(crate) const UNSENT_ON_DISCONNECT: &str = "daemon 연결 끊김 — 명령 미전송(재전송 안전)";

/// The message a command **that got no answer** because of a disconnect receives — the only axis that tells it apart from [`UNSENT_ON_DISCONNECT`] above is 「wire 에
/// 나갔나」 (did it go out on the wire), and that difference is exactly retry safety (for one that went out, there is no way to know whether the daemon executed it).
pub(crate) const SENT_OUTCOME_UNKNOWN: &str =
    "daemon 연결 끊김 — 명령 전송됨·응답 못 받음(결과 불명; 부작용 명령 맹목 재시도 금지)";

/// The prefix of a command that could not be handed to the link (the cause follows). A send failure, not a drop like
/// the two above — hence its own text.
pub const SEND_FAILED_PREFIX: &str = "명령 송신 실패";

/// The text of the older of two waiting round trips that share one request id — the transport fails both (M1 —
/// transport TRD §4-1); the newer one reads as a send failure (`RequestError::TagRetired`).
pub const PENDING_SUPERSEDED: &str = "중복 request_id — 옛 요청 취소";

/// `Err(NOT_CONNECTED)` unless the peer is live — "no connection takes commands now" (ADR-0195).
pub fn require_live(state: PeerState) -> Result<(), String> {
    if state.is_live() {
        Ok(())
    } else {
        Err(NOT_CONNECTED.to_owned())
    }
}

/// The text a failed `request` gives its caller, assuming [`require_live`] passed just before.
///
/// `QueueFull` and `TagRetired` read as a send failure with the transport's reason.
pub fn request_error_text(err: &RequestError) -> String {
    match err {
        RequestError::TimedOut { after } => reply_timeout_text(*after),
        RequestError::Disconnected { sent: false, .. } => UNSENT_ON_DISCONNECT.to_owned(),
        RequestError::Disconnected { sent: true, .. } => SENT_OUTCOME_UNKNOWN.to_owned(),
        RequestError::Superseded => PENDING_SUPERSEDED.to_owned(),
        RequestError::NotARequest => NO_REQUEST_ID.to_owned(),
        RequestError::QueueFull | RequestError::TagRetired => {
            format!("{SEND_FAILED_PREFIX}: {err}")
        }
    }
}

/// The text a failed `notify` gives its caller, assuming [`require_live`] passed just before.
pub fn send_error_text(err: &SendError) -> String {
    match err {
        SendError::Disconnected => UNSENT_ON_DISCONNECT.to_owned(),
        SendError::Closed => CHANNEL_GONE.to_owned(),
        SendError::QueueFull => format!("{SEND_FAILED_PREFIX}: {err}"),
    }
}

/// The text of a reply that did not come within `after` while the link stayed up.
pub fn reply_timeout_text(after: Duration) -> String {
    format!(
        "daemon reply timeout — 연결은 살아있으나 데몬이 응답을 보내지 않음({}s 초과)",
        after.as_secs()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use engram_dashboard_transport::{DisconnectCause, GaveUpReason, Generation};

    fn disconnected(sent: bool) -> RequestError {
        RequestError::Disconnected {
            generation: Generation(3),
            cause: DisconnectCause::PeerClosed,
            sent,
        }
    }

    #[test]
    fn only_a_live_peer_passes_the_pre_check_and_the_rest_get_not_connected() {
        assert_eq!(require_live(PeerState::Live), Ok(()));
        for state in [
            PeerState::Idle,
            PeerState::Dialing,
            PeerState::Handshaking,
            PeerState::Backoff,
            PeerState::GaveUp(GaveUpReason::BudgetExhausted),
            PeerState::GaveUp(GaveUpReason::Rejected),
            PeerState::Closed,
        ] {
            assert_eq!(
                require_live(state),
                Err(NOT_CONNECTED.to_owned()),
                "{state:?}"
            );
        }
    }

    #[test]
    fn request_failures_read_as_the_shell_texts_byte_for_byte() {
        assert_eq!(
            request_error_text(&disconnected(false)),
            UNSENT_ON_DISCONNECT
        );
        assert_eq!(
            request_error_text(&disconnected(true)),
            SENT_OUTCOME_UNKNOWN
        );
        assert_eq!(
            request_error_text(&RequestError::Superseded),
            PENDING_SUPERSEDED
        );
        assert_eq!(
            request_error_text(&RequestError::NotARequest),
            NO_REQUEST_ID
        );
        for err in [RequestError::QueueFull, RequestError::TagRetired] {
            let text = request_error_text(&err);
            assert_eq!(text, format!("{SEND_FAILED_PREFIX}: {err}"));
        }
    }

    #[test]
    fn a_request_timeout_reads_as_the_reply_timeout_text_of_the_frontend_command_path() {
        // Golden = the text `commands::agent::forward_to_daemon` gave before it called this function.
        assert_eq!(
            request_error_text(&RequestError::TimedOut {
                after: Duration::from_secs(30)
            }),
            "daemon reply timeout — 연결은 살아있으나 데몬이 응답을 보내지 않음(30s 초과)"
        );
    }

    #[test]
    fn send_failures_read_as_the_shell_texts_byte_for_byte() {
        assert_eq!(
            send_error_text(&SendError::Disconnected),
            UNSENT_ON_DISCONNECT
        );
        assert_eq!(send_error_text(&SendError::Closed), CHANNEL_GONE);
        assert_eq!(
            send_error_text(&SendError::QueueFull),
            format!("{SEND_FAILED_PREFIX}: {}", SendError::QueueFull)
        );
    }
}
