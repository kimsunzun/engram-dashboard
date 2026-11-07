//! Vocabulary of what goes upward — data types only, no logic.
//!
//! ★Each handle has exactly one upward line★ ([`Incoming`]) — packets and events flow through the same line. Having no side channel is the
//! decision (ADR-0177 decision 9): a signal not put into the stream the consumer **already reads** is the same as nonexistent if not heard.
//! The price = a consumer that wants only packets also uses one more layer of `match`.
//!
//! ★This list is the crate boundary★ (ADR-0182 「영향」 (Consequences)) — **agent errors and command failures are not here.** The upper layer
//! knows about them, and merging several sources is done later by the screen. If even one entry comes in on the grounds of 「이것도 같이 올리면 편하다」 (it would be convenient to raise this too),
//! that boundary breaks on the spot.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use crate::frame::CloseCode;
use crate::machine::{ConnectFailure, DisconnectCause, GaveUpReason, Generation};
use crate::wire::Wire;

/// Label the consumer attaches to a peer. ★This value is the event's source marker, and it later shows on screen as is★.
///
/// The crate does not interpret this string — it only checks whether two are equal.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PeerId(pub Arc<str>);

impl PeerId {
    pub fn new(name: impl AsRef<str>) -> Self {
        Self(Arc::from(name.as_ref()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Seed that scatters jitter per peer. ★Not random but a function of the name, so it is the same across re-runs★ (FNV-1a 64).
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

/// From which direction it was discarded.
///
/// ★There is no receiver-side branch★ — even when the upward line is full, the supervisor holds items in its own ordering area instead of discarding them
/// (`peer.rs` header 「순서 칸」 (ordering area) · TRD 3 §3-0 ②).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Could not send out what was meant to go to the peer. It includes not only queue saturation but also what was discarded because of a disconnect, and among those are
    /// notifications and fan-outs **cut off mid-write** (a request in the same situation is not included here; it wakes with an error).
    ///
    /// ★It is not 「확실히 잃었다」 (certainly lost) but 「쓰기가 `Ok` 로 끝나는 것을 못 봤다」 (did not see the write end in Ok) — the count can be inflated★. The place where it is inflated is
    /// a frame cut off mid-write — it may already have been in the adapter buffer, and on the WS adapter, the close
    /// sent when the link is discarded may push it out first (source reading — `ws.rs` 「통로 성질」 (link properties)). Under the same congestion that close
    /// can also be cut by the write deadline.
    ///
    /// A frame that [`crate::Registry::broadcast`] could not put in is counted once by that peer's supervisor (queue saturation · not in the operating phase).
    /// But a frame that could not be put in because the peer was not in the operating phase after the app called [`crate::Peer::close`] is not counted (`lib.rs`
    /// 「알려진 한계」 (known limits)) — a frame put into a peer still operating after close was called, where the queue then filled, is counted.
    ///
    /// ★It also includes what was not written because the bound generation was not the current one just before the write★ (TRD 3 §3-0 ③ — [`crate::Peer::notify_in`])
    /// — the connection was operating when it was queued, but it reconnected before the write. That report may come after the next connection's
    /// [`TransportEvent::Connected`] — what was discarded belongs to the previous connection. A call where the generation bound by `notify` · `notify_in` at the entrance
    /// was not operating and got `Err` ([`crate::SendError::Disconnected`]) is not counted — an `Err` received because the queue was full is
    /// counted (queue saturation). Fan-outs and resume calls cannot get the error to the caller, so an entrance rejection is counted too (above). A queue
    /// that filled in an ended generation only reports and does not cut the next connection.
    ///
    /// Conversely, what was not counted does not guarantee arrival either — the `Ok` of a write only reaches the transport buffer.
    Outbound,
}

/// One packet from the peer.
pub struct Arrival<W: Wire> {
    /// ★Source marker★ — which peer it is from.
    pub peer: PeerId,
    /// ★Generation boundary★ — which **connection** generation this piece belongs to. The handle stays alive across reconnects,
    /// so this is where the boundary shows.
    ///
    /// ★A stream fragment can arrive carrying an old generation number after a newer generation's [`TransportEvent::Connected`]★ — a fragment the resume
    /// window held goes upward when the window closes after reconnecting, but it carries the generation of the connection it arrived on. So once the decision to turn the stream layer on
    /// is made (today's two consumers do not use it — ADR-0310 decision 16), that receiver must not filter stream fragments by generation —
    /// filtering would discard fragments the window held and then raised. Generation filtering applies only to non-stream messages.
    ///
    /// ★To send an answer · request hanging on this message only over the connection it arrived on, pass this value to [`crate::Peer::notify_in`] ·
    /// [`crate::Peer::request_in`]★ — if it reconnected in between, it does not go over to the next connection and is not sent (TRD 3
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

/// Everything that comes up into the inbox ([`crate::Inbox`]).
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

/// What happened to the connection. ★All of them carry `peer`★.
///
/// The `generation` of the stream family is an **incarnation marker** (u32), a different thing from [`Generation`] (connection generation) —
/// the names are the same and easy to confuse, so it is quicker to remember that they are told apart by width.
/// ★`#[non_exhaustive]`★ — TRD §7 pinned it down as 「항목은 늘어난다」 (items grow) and this crate aims at reuse.
/// If it is not set now, the moment one more branch is added, every consumer's `match` breaks.
#[non_exhaustive]
pub enum TransportEvent<W: Wire> {
    Connected {
        peer: PeerId,
        generation: Generation,
    },
    /// ★Today's code has no such `cause` distinction — everything is one lump of 「끊김」 (disconnected)★.
    Disconnected {
        peer: PeerId,
        generation: Generation,
        cause: DisconnectCause,
    },
    /// ★Today the failure reason during a reconnect is left only as `debug!` and discarded★.
    ConnectFailed {
        peer: PeerId,
        attempt: u32,
        of: u32,
        cause: ConnectFailure,
        retry_in: Option<Duration>,
    },
    /// ★Terminal rejection ([`crate::RejectKind::Terminal`])★ — [`TransportEvent::GaveUp`] comes right after (the version
    /// rejection of ADR-0178 · the dialer's [`crate::DialFailure::Refused`] comes up here). A rejection that is retried does not come here; it goes
    /// as the `cause` of [`TransportEvent::ConnectFailed`].
    ///
    /// ★The popup text is a fixed local string chosen by `code` — not `reason`★ (TRD 3 §3-0 ⑤). `reason` may be text the peer
    /// gave, so it is neither used to decide nor used as screen text and goes only to details · the log — it is masked text
    /// ([`crate::mask_reason`] · [`crate::ConnectFailure::Rejected`]).
    Rejected {
        peer: PeerId,
        code: CloseCode,
        reason: String,
    },
    /// Input from which the shell decides popup eligibility (ADR-0180 decision 2).
    GaveUp { peer: PeerId, reason: GaveUpReason },
    /// ★This supervisor's last event and the last item of that handle's inbox ([`crate::Inbox`])★ — cleanup closes all links and loads everything held in the
    /// ordering area, then emits it at the very end and releases the supervisor's sending end, so the next `recv` on that inbox is `None`
    /// (TRD 3 §3-0 ②).
    ///
    /// `generation` = this supervisor's last connection generation (the floor value the registry gave if it never connected) · `cause` = the reason it ended.
    /// ★A peer that we dialed ([`crate::Registry::add`]) is always [`DisconnectCause::Explicit`]★ — the supervisor's only way to end is a close
    /// (the app closed it, or all its handles are gone, or the receiver closed the inbox). ★A peer that was accepted ([`crate::Registry::attach`]) carries
    /// the real reason★ — it does not redial, so losing the connection is the end: if it dropped while operating, the same reason as the preceding `Disconnected`;
    /// if it never reached the operating phase, the place where the handshake stopped (our handshake's rejection = `Explicit` · peer closed = `PeerClosed` ·
    /// read · write error = `ReadFailed` · `WriteFailed` · write passed its deadline = `WriteDeadline` · the peer could not
    /// finish by the auth deadline = `Idle`), and `Explicit` if the app · the receiver closed it.
    ///
    /// ★Do not tell supervisors apart by `generation` — the inbox does★ — the floor value is the generation the previous supervisor of the same label
    /// last drew, so for a supervisor that ended without ever connecting, this can be identical to the previous supervisor's. Each supervisor has
    /// its own inbox, so the two never mix in one inbox.
    ///
    /// ★It may not come★ — after the receiver closes or drops the inbox there is nowhere to load it (one already loaded before closing can still be taken out
    /// after closing). It also does not come when the supervisor ends without a normal cleanup —
    /// either cleanup went past its cap (`CLOSE_DRAIN_LIMIT` in `peer.rs`) against a receiver that stays alive and does not drain, and discarded this along with the rest
    /// (not counted and not reported), or the task ended abnormally (panic · runtime shutdown). Either way the sending
    /// end drops and a `None` without `Closed` signals the end — do not read it only as a cap overrun.
    ///
    /// The read-end / write-end split of a disconnect while operating ([`DisconnectCause::ReadFailed`] · [`DisconnectCause::WriteFailed`])
    /// is carried by the preceding [`TransportEvent::Disconnected`] — an accepted peer carries it here too (the daemon adapter tells the read
    /// end from the write end by this one reason — TRD 3 §3-0 ①).
    Closed {
        peer: PeerId,
        generation: Generation,
        cause: DisconnectCause,
    },
    /// Where and how many were lost and what was requested again. ★Today there is no sequence-hole check at all★.
    StreamGap {
        peer: PeerId,
        stream: W::StreamKey,
        generation: u32,
        expected: u64,
        got: u64,
        resumed_from: Option<u64>,
    },
    /// ★Not true/false but a **number**★ — the oldest sequence number valid now (ADR-0177 decision 9).
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
    /// Forgot this stream's ledger because of the stream-count cap.
    ///
    /// ★If the peer keeps inventing new keys, room runs out★ — if what was forgotten were not reported then, from that point
    /// that stream's duplicate removal and hole detection would silently restart from scratch. Fragments being held are not
    /// discarded and come up together with [`TransportEvent::StreamTruncated`].
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
    /// A reply came but **there is no waiting slot**.
    ///
    /// A reply that arrives late after expiry does not come here — that is quietly discarded by design (ADR-0181 decision 4). What
    /// comes here is **a pairing number we never asked for in the first place**, and it is either a consumer-side mistake (a command that expects a reply was pushed through
    /// `notify`/`broadcast`) or a number the peer made up.
    UnmatchedReply { peer: PeerId, tag: W::Tag },
    /// Number of discarded items — what is counted is defined by each branch of [`Direction`]. It comes up regardless of queue policy.
    /// ★This field exists so as not to create silent loss★.
    Dropped {
        peer: PeerId,
        direction: Direction,
        count: u64,
    },
    /// [`Wire::decode`] failed. Whether to disconnect is decided by [`crate::DecodeFailurePolicy`].
    ///
    /// `reason` = classification of the consumer's decode error ([`crate::wire::DecodeClass::class`]) — ★a fixed word that does not hold the received value★.
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

    /// Stable name for picking out the branch in logs · assertions.
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
        // All `#[non_exhaustive]` means is that this enum cannot be matched exhaustively outside the crate.
        // Here we only measure that the branch name comes out as a stable string — the consumer tells branches apart by it.
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
