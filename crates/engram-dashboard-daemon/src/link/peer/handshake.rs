//! The peer link's handshake — the `Hello` exchange (transport [`Handshake`]), one value per link for an accepted link and one per
//!   dialing handle (reused by each of its redials).
//!
//! - Dialer: sends its `Hello` first, then reads the acceptor's `Hello` → `Done`. Acceptor: reads the dialer's `Hello` → `Done`; its own
//!   `Hello` goes out as the link's first frame once it is up (the runtime sends it) — the shell-link pattern, since a transport handshake
//!   step that sends cannot also finish.
//! - Both sides check the other's `protocol_version` (unequal → terminal refusal, "version mismatch"; the acceptor's refusal carries its
//!   own `Hello` as the last frame so the dialer sees why) and the other's run against the link book ([`LinkBook::admit`] — a replaced or
//!   refused run → terminal refusal with a `Duplicate` last frame).
//! - What a handshake learns — the other side's run, or why it refused us — is left in [`HandshakeNotes`] for the runtime: the transport
//!   reports only a close code, and one code (`HANDSHAKE_REJECTED`) covers "not paired", a version mismatch and a duplicate. ★A refusal's
//!   last frame is best-effort★ — with none, a refusal reads as "not paired".

// ADR-0388, ADR-0389

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use engram_dashboard_base::sync::lock;
use engram_dashboard_transport::{Clock, Frame, Handshake, HandshakeStep, RejectKind};

use super::book::{LinkBook, Refusal};
use super::id::PeerId;
use super::wire::{decode_text, frame_of, Instance, PeerMsg, PEER_WIRE_VERSION};

/// What handshakes of one handle learned.
#[derive(Debug, Default)]
pub(crate) struct HandshakeNotes {
    inner: Mutex<Notes>,
}

#[derive(Debug, Default)]
struct Notes {
    /// The other side's run, one per finished handshake, in order — each `Connected` takes the oldest.
    runs: VecDeque<Instance>,
    /// Why the last attempt was refused, when the refusal said so.
    refusal: Option<Refusal>,
}

impl HandshakeNotes {
    /// The run of the link that just came up.
    pub(crate) fn take_run(&self) -> Option<Instance> {
        lock(&self.inner).runs.pop_front()
    }

    /// Why the dialing handle gave up — "not paired" unless a refusal frame said otherwise.
    pub(crate) fn refusal(&self) -> Refusal {
        lock(&self.inner).refusal.unwrap_or(Refusal::NotPaired)
    }

    /// Forgets why the last attempt was refused — the handle dials again after a re-arm, and a refusal without a frame then reads as
    ///   "not paired", not as the earlier reason.
    pub(crate) fn rearm(&self) {
        lock(&self.inner).refusal = None;
    }

    fn finished(&self, run: Instance) {
        let mut notes = lock(&self.inner);
        notes.runs.push_back(run);
        notes.refusal = None;
    }

    fn refused(&self, why: Refusal) {
        lock(&self.inner).refusal = Some(why);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Dial,
    Accept,
}

/// The handshake of one link (module head).
pub(crate) struct PeerHandshake {
    role: Role,
    node: PeerId,
    own: Instance,
    book: Arc<Mutex<LinkBook>>,
    clock: Arc<dyn Clock>,
    notes: Arc<HandshakeNotes>,
}

impl PeerHandshake {
    pub(crate) fn new(
        role: Role,
        node: PeerId,
        own: Instance,
        book: Arc<Mutex<LinkBook>>,
        clock: Arc<dyn Clock>,
        notes: Arc<HandshakeNotes>,
    ) -> Self {
        Self {
            role,
            node,
            own,
            book,
            clock,
            notes,
        }
    }

    fn hello(&self) -> Frame {
        frame_of(&PeerMsg::Hello {
            protocol_version: PEER_WIRE_VERSION,
            instance: self.own,
        })
    }

    fn refuse(&self, why: Refusal, last: Option<Frame>, reason: &str) -> HandshakeStep {
        self.notes.refused(why);
        HandshakeStep::Reject {
            kind: RejectKind::Terminal,
            last,
            reason: reason.to_string(),
        }
    }

    /// The other side's `Hello`: version, then the run.
    fn on_hello(&self, version: u32, run: Instance) -> HandshakeStep {
        if version != PEER_WIRE_VERSION {
            tracing::warn!(pc = %self.node, ours = PEER_WIRE_VERSION, theirs = version, "peer link refused — protocol version mismatch");
            let last = (self.role == Role::Accept).then(|| self.hello());
            return self.refuse(Refusal::VersionMismatch, last, "version mismatch");
        }
        let admitted = lock(&self.book).admit(&self.node, run, self.clock.now());
        if let Err(kind) = admitted {
            tracing::warn!(pc = %self.node, %run, "peer link refused — that PC's identity is in use by another running copy");
            let last = frame_of(&PeerMsg::Duplicate { kind });
            return self.refuse(Refusal::Duplicate(kind), Some(last), "duplicate identity");
        }
        self.notes.finished(run);
        HandshakeStep::Done
    }
}

impl Handshake for PeerHandshake {
    fn start(&self) -> HandshakeStep {
        match self.role {
            Role::Dial => HandshakeStep::Send(self.hello()),
            Role::Accept => HandshakeStep::Await,
        }
    }

    fn on_frame(&self, frame: &Frame) -> HandshakeStep {
        let msg = match frame {
            Frame::Text(text) => decode_text(text).ok(),
            Frame::Binary(_) | Frame::Keepalive => None,
        };
        match msg {
            Some(PeerMsg::Hello {
                protocol_version,
                instance,
            }) => self.on_hello(protocol_version, instance),
            // The acceptor's refusal says why; its close code decides (the transport contract for a refusal frame).
            Some(PeerMsg::Duplicate { kind }) if self.role == Role::Dial => {
                self.notes.refused(Refusal::Duplicate(kind));
                lock(&self.book).duplicate_reported(&self.node, kind);
                HandshakeStep::Await
            }
            other => {
                tracing::warn!(pc = %self.node, got = ?other.as_ref().map(PeerMsg::kind), "peer link refused — the first frame was not a Hello");
                HandshakeStep::Reject {
                    kind: RejectKind::Retryable,
                    last: None,
                    reason: "expected Hello".to_string(),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::peer::book::Dir;
    use crate::link::peer::wire::{encode_text, DuplicateKind};
    use engram_dashboard_transport::SystemClock;

    /// The frame an acceptor sends when it refuses a duplicate run.
    fn duplicate_frame(kind: DuplicateKind) -> Frame {
        frame_of(&PeerMsg::Duplicate { kind })
    }

    const A: PeerId = PeerId::from_bytes([1; 32]);
    const B: PeerId = PeerId::from_bytes([2; 32]);

    fn hello(version: u32, run: u128) -> Frame {
        Frame::Text(
            encode_text(&PeerMsg::Hello {
                protocol_version: version,
                instance: Instance::from_u128(run),
            })
            .unwrap(),
        )
    }

    fn shake(role: Role, book: &Arc<Mutex<LinkBook>>) -> (PeerHandshake, Arc<HandshakeNotes>) {
        let notes = Arc::new(HandshakeNotes::default());
        let hs = PeerHandshake::new(
            role,
            B,
            Instance::from_u128(1),
            book.clone(),
            Arc::new(SystemClock),
            notes.clone(),
        );
        (hs, notes)
    }

    fn tracked() -> Arc<Mutex<LinkBook>> {
        let mut book = LinkBook::new(A, 0);
        book.ensure(B);
        Arc::new(Mutex::new(book))
    }

    #[test]
    fn the_dialer_speaks_first_and_the_acceptor_waits() {
        let book = tracked();
        let (dial, _) = shake(Role::Dial, &book);
        let HandshakeStep::Send(Frame::Text(text)) = dial.start() else {
            panic!("the dialer sends its Hello")
        };
        assert!(matches!(
            decode_text(&text),
            Ok(PeerMsg::Hello { protocol_version: PEER_WIRE_VERSION, instance }) if instance == Instance::from_u128(1)
        ));
        let (accept, _) = shake(Role::Accept, &book);
        assert!(matches!(accept.start(), HandshakeStep::Await));
    }

    #[test]
    fn a_hello_of_this_version_finishes_and_leaves_the_run() {
        let book = tracked();
        for role in [Role::Dial, Role::Accept] {
            let (hs, notes) = shake(role, &book);
            assert!(matches!(
                hs.on_frame(&hello(PEER_WIRE_VERSION, 7)),
                HandshakeStep::Done
            ));
            assert_eq!(notes.take_run(), Some(Instance::from_u128(7)));
            assert_eq!(notes.take_run(), None);
        }
    }

    #[test]
    fn another_version_is_a_terminal_refusal_and_the_acceptor_says_its_own() {
        let book = tracked();
        let (accept, notes) = shake(Role::Accept, &book);
        let HandshakeStep::Reject { kind, last, .. } =
            accept.on_frame(&hello(PEER_WIRE_VERSION + 1, 7))
        else {
            panic!("refused")
        };
        assert_eq!(kind, RejectKind::Terminal);
        let Some(Frame::Text(text)) = last else {
            panic!("the acceptor's Hello goes as the last frame")
        };
        assert!(matches!(decode_text(&text), Ok(PeerMsg::Hello { .. })));
        assert_eq!(notes.refusal(), Refusal::VersionMismatch);

        // The dialer reading that Hello refuses for the same reason.
        let (dial, notes) = shake(Role::Dial, &book);
        assert!(matches!(
            dial.on_frame(&hello(PEER_WIRE_VERSION + 1, 7)),
            HandshakeStep::Reject {
                kind: RejectKind::Terminal,
                ..
            }
        ));
        assert_eq!(notes.refusal(), Refusal::VersionMismatch);
    }

    #[test]
    fn a_replaced_run_is_refused_with_a_duplicate_last_frame() {
        let book = tracked();
        let t = std::time::Instant::now();
        {
            let mut b = lock(&book);
            b.link_up(B, 90, Dir::Accepted, Instance::from_u128(5), false, t);
            b.link_down(90, t);
            b.link_up(B, 91, Dir::Accepted, Instance::from_u128(6), false, t);
        }
        let (accept, notes) = shake(Role::Accept, &book);
        let HandshakeStep::Reject { kind, last, .. } =
            accept.on_frame(&hello(PEER_WIRE_VERSION, 5))
        else {
            panic!("refused")
        };
        assert_eq!(kind, RejectKind::Terminal);
        assert_eq!(last, Some(duplicate_frame(DuplicateKind::Duplicate)));
        assert_eq!(
            notes.refusal(),
            Refusal::Duplicate(DuplicateKind::Duplicate)
        );
    }

    #[test]
    fn the_dialer_reads_a_duplicate_refusal_frame_and_waits_for_the_close() {
        let book = tracked();
        let (dial, notes) = shake(Role::Dial, &book);
        assert!(matches!(
            dial.on_frame(&duplicate_frame(DuplicateKind::Duplicate)),
            HandshakeStep::Await
        ));
        assert_eq!(
            notes.refusal(),
            Refusal::Duplicate(DuplicateKind::Duplicate)
        );
        assert_eq!(
            lock(&book).warning(&B).map(|w| (w.kind, w.reported)),
            Some((DuplicateKind::Duplicate, true))
        );
        // A later finished handshake clears it.
        assert!(matches!(
            dial.on_frame(&hello(PEER_WIRE_VERSION, 8)),
            HandshakeStep::Done
        ));
        assert_eq!(notes.refusal(), Refusal::NotPaired);
    }

    #[test]
    fn a_re_arm_forgets_the_earlier_refusal_reason() {
        let book = tracked();
        let (dial, notes) = shake(Role::Dial, &book);
        dial.on_frame(&hello(PEER_WIRE_VERSION + 1, 7));
        assert_eq!(notes.refusal(), Refusal::VersionMismatch);
        // Re-armed, the next attempt is refused at the gate without a frame — that reads as "not paired".
        notes.rearm();
        assert_eq!(notes.refusal(), Refusal::NotPaired);
    }

    #[test]
    fn anything_but_a_hello_first_is_refused_and_a_silent_refusal_reads_as_not_paired() {
        let book = tracked();
        let (accept, notes) = shake(Role::Accept, &book);
        for frame in [
            Frame::Text("{}".to_string()),
            Frame::Binary(vec![1]),
            duplicate_frame(DuplicateKind::Suspected),
        ] {
            assert!(matches!(
                accept.on_frame(&frame),
                HandshakeStep::Reject { .. }
            ));
        }
        assert_eq!(notes.refusal(), Refusal::NotPaired);
        assert_eq!(notes.take_run(), None);
    }
}
