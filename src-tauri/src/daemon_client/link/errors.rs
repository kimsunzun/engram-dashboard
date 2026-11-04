//! Transport send and request failures as the texts the old driver gives its callers (TRD S21 trd-3 §6-2 U2).
//!
//! ★Byte-identical to the old driver★ — callers tell outcomes apart by these strings (the frontend, the LLM surface,
//! `daemon_client::tests` `DisconnectBranch`). The old driver's constants are imported in place; the reply-timeout text
//! lives here and the frontend command path (`commands::agent`) calls it. Failures the old driver never had (a full
//! queue, a retired request id) read as its send failure with the transport's reason.
//!
//! ★Pre-check first★ — [`RequestError::Disconnected`] with `sent: false` covers both "the peer was not live when
//! called" and "it dropped before the command reached the wire". The old driver gives those two different texts
//! ([`NOT_CONNECTED`] · [`UNSENT_ON_DISCONNECT`]), so a caller runs [`require_live`] before `request` / `notify` and
//! maps what the transport returns afterwards with [`request_error_text`] / [`send_error_text`].
// ADR-0195

use std::time::Duration;

use engram_dashboard_transport::{PeerState, RequestError, SendError};

use crate::daemon_client::connection::{
    PENDING_SUPERSEDED, SEND_FAILED_PREFIX, SENT_OUTCOME_UNKNOWN, UNSENT_ON_DISCONNECT,
};
use crate::daemon_client::{CHANNEL_GONE, NOT_CONNECTED, NO_REQUEST_ID};

/// `Err(NOT_CONNECTED)` unless the peer is live — the old driver's "no socket takes commands now" (ADR-0195).
pub fn require_live(state: PeerState) -> Result<(), String> {
    if state.is_live() {
        Ok(())
    } else {
        Err(NOT_CONNECTED.to_owned())
    }
}

/// The text a failed `request` gives its caller, assuming [`require_live`] passed just before.
///
/// `QueueFull` and `TagRetired` have no counterpart in the old driver (its queue waited instead of failing, and a
/// reused request id took the slot over) — they read as a send failure with the transport's reason.
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
    fn request_failures_read_as_the_old_driver_texts_byte_for_byte() {
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
    fn send_failures_read_as_the_old_driver_texts_byte_for_byte() {
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
