//! Transport send and request failures as the texts the shell gives its callers (TRD S21 trd-3 §6-2 U2). The texts are a contract: callers (the frontend, the LLM surface) tell outcomes apart by these strings, so each stays one fixed English cause that the frontend shows inside its translated head (TRD S38 D1). Failures the shell never had before (a full queue, a retired request id) read as its send failure with the transport's reason.
//! Pre-check first: [`RequestError::Disconnected`] with `sent: false` covers both "the peer was not live when called" and "it dropped before the command reached the wire" — two different texts ([`NOT_CONNECTED`] · [`UNSENT_ON_DISCONNECT`]), so a caller runs [`require_live`] before `request` / `notify` and maps what the transport returns afterwards with [`request_error_text`] / [`send_error_text`].
// ADR-0195

use std::time::Duration;

use engram_dashboard_transport::{PeerState, RequestError, SendError};

use crate::daemon_client::{CHANNEL_GONE, NOT_CONNECTED, NO_REQUEST_ID};

/// The text of a command that never reached the wire because the link dropped. "Safe to resend" holds on this path only — one that went out has an unknown outcome ([`SENT_OUTCOME_UNKNOWN`]).
pub(crate) const UNSENT_ON_DISCONNECT: &str = "daemon disconnected — command not sent (safe to resend)";

/// The message of a command that got no answer because of a disconnect — the only axis telling it from [`UNSENT_ON_DISCONNECT`] is "did it go out on the wire", which is exactly retry safety (for one that went out there is no way to know whether the daemon executed it).
pub(crate) const SENT_OUTCOME_UNKNOWN: &str =
    "daemon disconnected — command sent, no reply received (outcome unknown; do not blindly retry side-effecting commands)";

/// The prefix of a command that could not be handed to the link (the cause follows) — a send failure, not a drop like the two above, hence its own text.
pub const SEND_FAILED_PREFIX: &str = "failed to send command";

/// The text of the older of two waiting round trips sharing one request id — the transport fails both (M1 — transport TRD §4-1); the newer one reads as a send failure (`RequestError::TagRetired`).
pub const PENDING_SUPERSEDED: &str = "duplicate request_id — the old request was cancelled";

/// ADR-0195
pub fn require_live(state: PeerState) -> Result<(), String> {
    if state.is_live() {
        Ok(())
    } else {
        Err(NOT_CONNECTED.to_owned())
    }
}

/// The text a failed `request` gives its caller, assuming [`require_live`] passed just before; `QueueFull` and `TagRetired` read as a send failure with the transport's reason.
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
        "daemon reply timeout — the connection is alive but the daemon sent no reply (over {}s)",
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
            "daemon reply timeout — the connection is alive but the daemon sent no reply (over 30s)"
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
