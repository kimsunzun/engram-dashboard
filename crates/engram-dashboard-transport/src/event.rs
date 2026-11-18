//! Vocabulary of what goes upward — data types only. ★Each handle has exactly one upward line ([`Incoming`])★ — packets and events share it; no side channel (ADR-0177 decision 9): a signal
//! outside the stream the consumer already reads goes unheard. Price: a packets-only consumer needs one more `match`. ★This list is the crate boundary★ (ADR-0182 「Impact」) —
//! **agent errors and command failures are not here** (upper layers know them; the screen merges sources); admitting one entry 「because it would be convenient」 breaks that boundary.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use crate::frame::CloseCode;
use crate::machine::{ConnectFailure, DisconnectCause, GaveUpReason, Generation};
use crate::wire::Wire;

/// Label the consumer attaches to a peer — ★the event's source marker, shown on screen as is★. The crate only compares it for equality.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PeerId(pub Arc<str>);

impl PeerId {
    pub fn new(name: impl AsRef<str>) -> Self {
        Self(Arc::from(name.as_ref()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Seed scattering jitter per peer — ★a function of the name (FNV-1a 64), so the same across re-runs★.
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

/// From which direction it was discarded. ★There is no receiver-side branch★ — a full upward line makes the supervisor hold items in its ordering area, not discard (`peer.rs` header · TRD 3 §3-0 ②).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Could not send out what was meant for the peer: queue saturation, discards because of a disconnect, and notifications · fan-outs **cut off mid-write** (a request in that case wakes with
    /// an error and is not counted). ★Not 「certainly lost」 but 「did not see the write end in `Ok`」 — the count can be inflated★: a frame cut mid-write may already be in the adapter buffer, and on
    /// WS the close sent on link discard may push it out first (`ws.rs` 「link properties」; that close can also be cut by the write deadline).
    ///
    /// [`crate::Registry::broadcast`] failures are counted once by that peer's supervisor (queue saturation · not operating). ★Not counted: frames refused because the peer was not operating after
    /// the app called [`crate::Peer::close`]★ (`lib.rs` 「known limits」); frames put in before that and then overflowing are counted. ★Also counted: frames not written because the bound generation
    /// was not current just before the write★ (TRD 3 §3-0 ③ — [`crate::Peer::notify_in`]); that report may arrive after the next connection's [`TransportEvent::Connected`]. An entry `Err` the
    /// caller received ([`crate::SendError::Disconnected`]) is not counted, but a queue-full `Err` is; fan-outs and resumes cannot return the error, so entry rejection is counted for them. A queue
    /// filled in an ended generation only reports and does not cut the next connection. Not counted ≠ delivered — an `Ok` write reaches only the transport buffer.
    Outbound,
}

pub struct Arrival<W: Wire> {
    /// ★Source marker★ — which peer it is from.
    pub peer: PeerId,
    /// ★Generation boundary★ — which **connection** generation this piece belongs to (the handle outlives reconnects).
    /// ★A stream fragment can carry an old generation after a newer generation's [`TransportEvent::Connected`]★ — a fragment the resume window held goes up on window close after reconnecting, stamped
    /// with the connection it arrived on. So once the stream layer is on (today's two consumers do not — ADR-0310 decision 16), the receiver must not filter stream fragments by generation (it
    /// would drop held fragments); filter only non-stream messages.
    /// ★To answer or request only on this message's connection, pass this to [`crate::Peer::notify_in`] · [`crate::Peer::request_in`]★ — after a reconnect nothing is sent (TRD 3 §3-0 ③).
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

/// What happened to the connection; ★all carry `peer`★. The stream family's `generation` is an **incarnation marker** (u32), not [`Generation`] (connection generation) — same name, told apart by
/// width. ★`#[non_exhaustive]`★ (TRD §7 「items grow」, crate aims at reuse): a new branch otherwise breaks every consumer's `match`.
#[non_exhaustive]
pub enum TransportEvent<W: Wire> {
    Connected {
        peer: PeerId,
        generation: Generation,
    },
    Disconnected {
        peer: PeerId,
        generation: Generation,
        cause: DisconnectCause,
    },
    ConnectFailed {
        peer: PeerId,
        attempt: u32,
        of: u32,
        cause: ConnectFailure,
        retry_in: Option<Duration>,
    },
    /// ★Terminal rejection ([`crate::RejectKind::Terminal`])★ — [`TransportEvent::GaveUp`] follows (ADR-0178 version rejection · the dialer's [`crate::DialFailure::Refused`]); a retried rejection
    /// is the `cause` of [`TransportEvent::ConnectFailed`]. ★The popup text is a fixed local string chosen by `code`, not `reason`★ (TRD 3 §3-0 ⑤): `reason` may be peer-given, goes only to
    /// details · the log, and is masked ([`crate::mask_reason`] · [`crate::ConnectFailure::Rejected`]).
    Rejected {
        peer: PeerId,
        code: CloseCode,
        reason: String,
    },
    GaveUp { peer: PeerId, reason: GaveUpReason },
    /// ★This supervisor's last event and the last item of its inbox ([`crate::Inbox`])★ — cleanup loads everything held in the ordering area, emits this last and releases the sending end, so
    /// the next `recv` is `None` (TRD 3 §3-0 ②). `generation` = this supervisor's last connection generation (the registry's floor if it never connected); `cause` = why it ended.
    /// ★A dialed peer ([`crate::Registry::add`]) is always [`DisconnectCause::Explicit`]★ (its only end is a close). ★An accepted peer ([`crate::Registry::attach`]) carries the real reason★ (it does not
    /// redial): a disconnect during operation = the preceding `Disconnected`'s cause (read/write end split — the daemon adapter uses it, TRD 3 §3-0 ①); a handshake that never completed = where it
    /// stopped (our rejection `Explicit` · peer closed `PeerClosed` · read/write error `ReadFailed`/`WriteFailed` · write past deadline `WriteDeadline` · peer silent past the auth deadline `Idle`);
    /// `Explicit` if the app · receiver closed it.
    ///
    /// ★Do not tell supervisors apart by `generation` — the inbox does★ (a supervisor that never connected carries the previous one's floor; each has its own inbox). ★It may not come★: after the
    /// receiver closes/drops the inbox (one already loaded can still be taken), or when the supervisor ends without normal cleanup (cleanup past `CLOSE_DRAIN_LIMIT` in `peer.rs` against a
    /// non-draining receiver — uncounted, unreported — or panic · runtime shutdown); then a `None` without `Closed` signals the end.
    Closed {
        peer: PeerId,
        generation: Generation,
        cause: DisconnectCause,
    },
    StreamGap {
        peer: PeerId,
        stream: W::StreamKey,
        generation: u32,
        expected: u64,
        got: u64,
        resumed_from: Option<u64>,
    },
    /// ★Not true/false but a **number**★ — the oldest valid sequence number now (ADR-0177 decision 9).
    StreamTruncated {
        peer: PeerId,
        stream: W::StreamKey,
        generation: u32,
        oldest_valid: u64,
    },
    /// The incarnation changed — the sequence cross-check was reset.
    StreamGenerationChanged {
        peer: PeerId,
        stream: W::StreamKey,
        from: u32,
        to: u32,
        /// Number of fragments held and then discarded for the old incarnation. Under the new incarnation those numbers have no meaning.
        discarded: u64,
    },
    /// Forgot this stream's ledger because of the stream-count cap. ★If the peer keeps inventing keys, room runs out★ — unreported, dedup and hole detection would silently restart from scratch. Held
    /// fragments are not discarded; they come up with [`TransportEvent::StreamTruncated`].
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
    /// A reply came but **there is no waiting slot**. A late reply after expiry does not come here (quietly discarded by design — ADR-0181 decision 4); this is **a number we never asked for**: a consumer
    /// mistake (a reply-expecting command pushed through `notify`/`broadcast`) or a number the peer made up.
    UnmatchedReply { peer: PeerId, tag: W::Tag },
    /// Number of discarded items (what counts is defined per [`Direction`] branch); raised regardless of queue policy. ★Exists so as not to create silent loss★.
    Dropped {
        peer: PeerId,
        direction: Direction,
        count: u64,
    },
    /// [`Wire::decode`] failed; whether to disconnect is [`crate::DecodeFailurePolicy`]. `reason` = classification of the consumer's decode error ([`crate::wire::DecodeClass::class`]) — ★a fixed word, never the received value★.
    DecodeFailed {
        peer: PeerId,
        generation: Generation,
        reason: &'static str,
    },
    /// One `In` was reported as both a reply and a stream fragment — a signal of a [`Wire`] contract violation.
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
        // `#[non_exhaustive]` only blocks exhaustive matching outside the crate; here we measure that the branch name is a stable string the consumer tells branches apart by.
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
