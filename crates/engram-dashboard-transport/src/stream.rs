//! Stream lifecycle management — ★pure layer★. Generation comparison · sequence-number dedup · hole detection · the 「유효 하한」 (valid floor).
//!
//! ★This file has no tokio, no socket, and no workspace crate★ — the gate measures that (canonical =
//! the `lib.rs` header). Time is received **as an argument** (the same idiom as `machine.rs`).
//!
//! ★It looks only at the incoming side★ — the outgoing stream (where the daemon stamps sequence numbers) is outside this crate.
//!
//! ## Why a re-request and its answer must be paired (do not revive)
//!
//! **Edition 1** held 「재요청을 냈다」 (a re-request was sent) as just one true/false. Then, at the moment a piece the peer had already
//! sent before receiving the re-request (pipelining — our output hot path does exactly that) arrived, it read it as
//! 「재요청에 대한 답이 또 구멍이다」 (the answer to the re-request is a hole again) and folded into a truncation, and after that all the replay the peer actually sent
//! became 「지나간 번호」 (a number already passed) and was dropped.
//!
//! **Edition 2** **dropped** that piece. That was worse — ★a daemon whose rewind buffer overflowed by definition
//! cannot give `after+1`★ (that is the **main path** of truncation). In that case, until the window closed, what arrived
//! vanished with neither event nor delivery, and the `oldest_valid` raised after that was a **number late by the window length**. That is, 「버리고 값도
//! 틀리게 신고」 (drop it and report the wrong value too).
//!
//! **Now** it is held. An out-of-order piece that arrives while waiting for the re-request's answer goes into a **bounded buffer**, and
//! when the window closes (deadline exceeded · buffer saturated · stream evicted · the caller's teardown) it is **sent out in order** and its first number is raised as
//! `oldest_valid`. So that value is true — because we really deliver from that number.
//! If the answer comes properly (= `after+1` arrives), what was held is dropped: the peer's replay carries that span again.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::hash::Hash;
use std::time::{Duration, Instant};

/// The marker of one stream piece.
///
/// ★`generation` is an **incarnation marker** and is different from [`crate::Generation`] (connection generation)★ — comparison is
/// **match/mismatch only** and does not derive "newer" from magnitude (ADR-0163). The width differs too (u32 vs u64).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamMark<K> {
    pub key: K,
    pub generation: u32,
    pub seq: u64,
}

/// The decision made after seeing one piece.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict<M> {
    /// It continues — send it up as is.
    Deliver(M),
    /// A sequence number already passed (or a number already held) — dropped.
    Duplicate,
    /// The incarnation changed — reset the sequence comparison and send up. `discarded` = the number of pieces that were held under the old incarnation
    /// and dropped (those numbers have no meaning in the new incarnation).
    GenerationChanged {
        from: u32,
        to: u32,
        msg: M,
        discarded: usize,
    },
    /// A hole. ★This piece is **held**★ — sending terminal bytes up out of order desyncs the ANSI parser state
    /// and breaks the screen. Ask again from after `resume_from`.
    Gap {
        expected: u64,
        got: u64,
        resume_from: u64,
    },
    /// Arrived while waiting for the answer to an already sent re-request — held. **Not a loss.**
    Held { awaiting_after: u64 },
    /// The window closed — send out what was held in order and continue.
    ///
    /// ★`oldest_valid` is the first number of `flushed`★ — it is true because we really deliver from that number.
    Truncated { oldest_valid: u64, flushed: Vec<M> },
}

impl<M> Verdict<M> {
    /// The pieces to send up.
    pub fn into_delivered(self) -> Vec<M> {
        match self {
            Verdict::Deliver(msg) => vec![msg],
            Verdict::GenerationChanged { msg, .. } => vec![msg],
            Verdict::Truncated { flushed, .. } => flushed,
            Verdict::Duplicate | Verdict::Gap { .. } | Verdict::Held { .. } => Vec::new(),
        }
    }
}

/// The result of folding one re-request whose deadline passed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Truncation<K, M> {
    pub key: K,
    pub generation: u32,
    pub oldest_valid: u64,
    pub flushed: Vec<M>,
}

/// One stream forgotten to make room.
///
/// ★It does not vanish silently★ — the pieces it was holding come out as `flushed`, and the caller reports it as an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evicted<K, M> {
    pub key: K,
    pub generation: u32,
    pub last_seq: u64,
    /// The pieces it was holding. If not empty, `oldest_valid` is its first number.
    pub flushed: Vec<M>,
    pub oldest_valid: Option<u64>,
}

/// One re-request that was sent out.
#[derive(Debug)]
struct Resume<M> {
    /// Asked for everything **after** this number. The first piece of the answer is `after + 1`.
    after: u64,
    /// When it was asked. Updated when sent out again after a reconnect.
    asked_at: Instant,
    /// The out-of-order pieces that arrived while waiting for the answer. When the window closes they go out in this order.
    held: BTreeMap<u64, M>,
}

#[derive(Debug)]
struct Track<M> {
    generation: u32,
    last_seq: u64,
    resume: Option<Resume<M>>,
}

impl<M> Track<M> {
    /// Takes out what was held — `(번호 순서대로, 첫 번호, 끝 번호)`.
    ///
    /// ★Progress is pushed to the last number★ — the held numbers may be sparse (a piece after the hole came backwards · only part of the replay
    /// came) and there is no guarantee that the piece that made the window close is the largest of them. If pushed to less than that, a number already sent out
    /// comes again as replay and is delivered once more against the order.
    fn drain_held(&mut self) -> (Vec<M>, Option<u64>, Option<u64>) {
        match self.resume.take() {
            Some(resume) => {
                let oldest = resume.held.keys().next().copied();
                let newest = resume.held.keys().next_back().copied();
                let flushed: Vec<M> = resume.held.into_values().collect();
                (flushed, oldest, newest)
            }
            None => (Vec::new(), None, None),
        }
    }
}

/// Holds 「마지막 세대 · 마지막 순번 · 내보낸 재요청」 (the last generation · the last sequence number · the re-request sent out) for each stream.
///
/// ★It is not erased even when the connection is cut★ — when it re-attaches, that last sequence number is the resume point, so if erased,
/// a reconnect would always receive from the start. Pending re-requests and held pieces also survive
/// and go out again through [`Streams::pending_resumes`].
///
/// ★It is the only collection whose size the peer decides★ — so there are two limits: the number of streams (`max_streams`) and
/// the number of pieces held per stream (`resume_buffer`). The caller may put on a further limit in its own unit (the driving layer uses bytes)
/// ([`Streams::observe_capped`]).
#[derive(Debug)]
pub struct Streams<K, M> {
    seen: HashMap<K, Track<M>>,
    /// ★Recently-used order★ — the back is the most recent. Eviction is done from the front (LRU).
    ///
    /// If kept in registration order, the **longest-living = most active** stream would become the first victim.
    order: VecDeque<K>,
    max_streams: usize,
    resume_buffer: usize,
    resume_timeout: Duration,
    evicted: Vec<Evicted<K, M>>,
}

impl<K: Eq + Hash + Clone, M> Streams<K, M> {
    pub fn new(max_streams: usize, resume_buffer: usize, resume_timeout: Duration) -> Self {
        Self {
            seen: HashMap::new(),
            order: VecDeque::new(),
            max_streams: max_streams.max(1),
            resume_buffer: resume_buffer.max(1),
            resume_timeout,
            evicted: Vec::new(),
        }
    }

    /// Looks at one piece. The decision is [`Verdict`] and the caller carries out event emission · re-request · delivery.
    pub fn observe(&mut self, mark: &StreamMark<K>, msg: M, now: Instant) -> Verdict<M> {
        self.observe_capped(mark, msg, now, false)
    }

    /// The edition of [`Streams::observe`] with the caller's hold limit added. `over_cap` = 「이 조각까지 붙들면 부르는 쪽이
    /// 재는 상한(구동 층에서는 바이트)을 넘는다」 (if held up to this piece, it exceeds the limit the caller measures (bytes in the driving layer)) — if true, we treat it as no room to hold. If a window is open,
    /// it closes with the same truncation as when `resume_buffer` of [`Self::new`] is full, and for a hole that would open a new window, it does not open the window and continues from this
    /// piece, reporting what came before it as a truncation ([`Verdict::Truncated`] — no re-request is sent).
    pub fn observe_capped(
        &mut self,
        mark: &StreamMark<K>,
        msg: M,
        now: Instant,
        over_cap: bool,
    ) -> Verdict<M> {
        if !self.seen.contains_key(&mark.key) {
            self.admit(mark);
            return Verdict::Deliver(msg);
        }
        self.touch(&mark.key);
        let resume_timeout = self.resume_timeout;
        let resume_buffer = self.resume_buffer;
        let track = self.seen.get_mut(&mark.key).expect("checked above");

        if track.generation != mark.generation {
            let from = track.generation;
            let (stale, ..) = track.drain_held();
            track.generation = mark.generation;
            track.last_seq = mark.seq;
            return Verdict::GenerationChanged {
                from,
                to: mark.generation,
                msg,
                discarded: stale.len(),
            };
        }

        if mark.seq <= track.last_seq {
            return Verdict::Duplicate;
        }

        if mark.seq == track.last_seq + 1 {
            track.last_seq = mark.seq;
            // ★The answer came★ — what was held is dropped. The peer's replay carries that span again so it is not a loss,
            //   and if sent out here, the same piece would instead be delivered twice.
            track.resume = None;
            return Verdict::Deliver(msg);
        }

        let Some(resume) = track.resume.as_mut() else {
            // ★If there is no room to hold from the start, do not open the window★ — if opened, this one frame stays over the caller's limit
            //   until the window closes.
            if over_cap {
                track.last_seq = mark.seq;
                return Verdict::Truncated {
                    oldest_valid: mark.seq,
                    flushed: vec![msg],
                };
            }
            let resume_from = track.last_seq;
            let mut held = BTreeMap::new();
            held.insert(mark.seq, msg);
            track.resume = Some(Resume {
                after: resume_from,
                asked_at: now,
                held,
            });
            return Verdict::Gap {
                expected: resume_from + 1,
                got: mark.seq,
                resume_from,
            };
        };

        if resume.held.contains_key(&mark.seq) {
            return Verdict::Duplicate;
        }
        resume.held.insert(mark.seq, msg);
        let expired = now.saturating_duration_since(resume.asked_at) >= resume_timeout;
        let full = resume.held.len() >= resume_buffer || over_cap;
        if !expired && !full {
            let awaiting_after = resume.after;
            return Verdict::Held { awaiting_after };
        }
        let (flushed, oldest, newest) = track.drain_held();
        let oldest_valid = oldest.expect("붙든 것이 하나는 있다");
        track.last_seq = newest.expect("붙든 것이 하나는 있다");
        Verdict::Truncated {
            oldest_valid,
            flushed,
        }
    }

    /// ★The path that runs even without arrivals★ — folds re-requests whose deadline passed.
    ///
    /// Without this, if the stream goes quiet after a hole (the ordinary situation where the turn ended), the re-request stays pending **forever**
    /// and held pieces never go out either. [`Streams::next_resume_deadline`] tells that time.
    pub fn expire_resumes(&mut self, now: Instant) -> Vec<Truncation<K, M>> {
        let timeout = self.resume_timeout;
        let due: Vec<K> = self
            .seen
            .iter()
            .filter(|(_, track)| {
                track
                    .resume
                    .as_ref()
                    .is_some_and(|r| now.saturating_duration_since(r.asked_at) >= timeout)
            })
            .map(|(key, _)| key.clone())
            .collect();
        due.into_iter()
            .filter_map(|key| self.collapse(key))
            .collect()
    }

    /// Folds every open window regardless of deadline — the caller calls it just before dropping this ledger (teardown in the driving layer).
    /// If not called, held pieces vanish with the ledger with neither delivery nor truncation report.
    pub fn close_resumes(&mut self) -> Vec<Truncation<K, M>> {
        let open: Vec<K> = self
            .order
            .iter()
            .filter(|key| self.seen.get(*key).is_some_and(|t| t.resume.is_some()))
            .cloned()
            .collect();
        open.into_iter()
            .filter_map(|key| self.collapse(key))
            .collect()
    }

    fn collapse(&mut self, key: K) -> Option<Truncation<K, M>> {
        let track = self.seen.get_mut(&key)?;
        let generation = track.generation;
        let (flushed, oldest, newest) = track.drain_held();
        let oldest_valid = oldest?;
        track.last_seq = track.last_seq.max(newest?);
        Some(Truncation {
            key,
            generation,
            oldest_valid,
            flushed,
        })
    }

    /// The earliest re-request deadline. `None` means there is no pending re-request.
    pub fn next_resume_deadline(&self) -> Option<Instant> {
        self.seen
            .values()
            .filter_map(|track| {
                track
                    .resume
                    .as_ref()
                    .map(|r| r.asked_at + self.resume_timeout)
            })
            .min()
    }

    /// Collects the streams forgotten to make room.
    pub fn drain_evicted(&mut self) -> Vec<Evicted<K, M>> {
        std::mem::take(&mut self.evicted)
    }

    fn touch(&mut self, key: &K) {
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            let k = self.order.remove(pos).expect("position 이 준 자리");
            self.order.push_back(k);
        }
    }

    fn admit(&mut self, mark: &StreamMark<K>) {
        while self.order.len() >= self.max_streams {
            let Some(victim) = self.order.pop_front() else {
                break;
            };
            if let Some(mut track) = self.seen.remove(&victim) {
                let generation = track.generation;
                let last_seq = track.last_seq;
                let (flushed, oldest_valid, _) = track.drain_held();
                self.evicted.push(Evicted {
                    key: victim,
                    generation,
                    last_seq,
                    flushed,
                    oldest_valid,
                });
            }
        }
        self.order.push_back(mark.key.clone());
        self.seen.insert(
            mark.key.clone(),
            Track {
                generation: mark.generation,
                last_seq: mark.seq,
                resume: None,
            },
        );
    }

    /// All re-requests that have not yet got an answer — `(스트림, 그 번호 뒤부터)`.
    ///
    /// ★If these are not sent out again after a reconnect, the hole stays forever★.
    pub fn pending_resumes(&self) -> Vec<(K, u64)> {
        self.order
            .iter()
            .filter_map(|key| {
                let track = self.seen.get(key)?;
                track.resume.as_ref().map(|r| (key.clone(), r.after))
            })
            .collect()
    }

    /// Winds the pending re-requests' clock again. Call it right after actually sending out the re-request.
    pub fn rearm_resumes(&mut self, now: Instant) {
        for track in self.seen.values_mut() {
            if let Some(resume) = track.resume.as_mut() {
                resume.asked_at = now;
            }
        }
    }

    /// Pushes back the requested time of pending re-requests by `by` — called when the caller could not read answers in the meantime (backpressure
    /// in the driving layer) so as to subtract that time from the deadline. If not pushed, the deadline runs out while we had blocked, and a hole that could have been filled
    /// is folded into a truncation.
    pub fn shift_resumes(&mut self, by: Duration) {
        for track in self.seen.values_mut() {
            if let Some(resume) = track.resume.as_mut() {
                resume.asked_at += by;
            }
        }
    }

    /// One piece being held. Used when the caller replaces its own marker on a piece it just held (the driving layer counts
    /// that piece's byte share as the window share — `budget.rs`).
    pub fn held_mut(&mut self, key: &K, seq: u64) -> Option<&mut M> {
        self.seen.get_mut(key)?.resume.as_mut()?.held.get_mut(&seq)
    }

    /// The last sequence number resumed so far.
    pub fn last_seq(&self, key: &K) -> Option<u64> {
        self.seen.get(key).map(|t| t.last_seq)
    }

    /// The incarnation marker known now.
    pub fn generation(&self, key: &K) -> Option<u32> {
        self.seen.get(key).map(|t| t.generation)
    }

    /// Forgets this stream (when the consumer ends its subscription).
    pub fn forget(&mut self, key: &K) {
        if self.seen.remove(key).is_some() {
            self.order.retain(|k| k != key);
        }
    }

    pub fn len(&self) -> usize {
        self.seen.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESUME_TIMEOUT: Duration = Duration::from_secs(2);

    fn streams() -> Streams<u8, u64> {
        Streams::new(1024, 256, RESUME_TIMEOUT)
    }

    fn mark(key: u8, generation: u32, seq: u64) -> StreamMark<u8> {
        StreamMark {
            key,
            generation,
            seq,
        }
    }

    /// The piece's payload is set to its own sequence number — the delivery order can be asserted as is.
    fn see(s: &mut Streams<u8, u64>, key: u8, gen: u32, seq: u64, now: Instant) -> Verdict<u64> {
        s.observe(&mark(key, gen, seq), seq, now)
    }

    #[test]
    fn first_chunk_of_a_stream_is_accepted_whatever_its_seq() {
        let mut s = streams();
        let t0 = Instant::now();
        assert_eq!(see(&mut s, 1, 7, 900, t0), Verdict::Deliver(900));
        assert_eq!(s.last_seq(&1), Some(900));
        assert_eq!(s.generation(&1), Some(7));
    }

    #[test]
    fn contiguous_chunks_deliver() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        assert_eq!(see(&mut s, 1, 7, 2, t0), Verdict::Deliver(2));
        assert_eq!(see(&mut s, 1, 7, 3, t0), Verdict::Deliver(3));
        assert_eq!(s.last_seq(&1), Some(3));
    }

    #[test]
    fn already_seen_seq_is_dropped() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 5, t0);
        assert_eq!(see(&mut s, 1, 7, 5, t0), Verdict::Duplicate);
        assert_eq!(see(&mut s, 1, 7, 4, t0), Verdict::Duplicate);
    }

    #[test]
    fn gap_is_detected_and_asks_from_the_last_good_seq() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        assert_eq!(
            see(&mut s, 1, 7, 5, t0),
            Verdict::Gap {
                expected: 2,
                got: 5,
                resume_from: 1,
            }
        );
        assert_eq!(s.last_seq(&1), Some(1), "구멍 조각은 진도를 밀지 않는다");
    }

    #[test]
    fn chunks_already_in_flight_do_not_look_like_truncation() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        assert!(matches!(see(&mut s, 1, 7, 5, t0), Verdict::Gap { .. }));
        assert_eq!(
            see(&mut s, 1, 7, 6, t0 + Duration::from_millis(1)),
            Verdict::Held { awaiting_after: 1 }
        );
        assert_eq!(s.last_seq(&1), Some(1));
    }

    #[test]
    fn the_replayed_range_is_delivered_even_after_later_chunks_arrived_first() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 1, 7, 5, t0);
        see(&mut s, 1, 7, 6, t0);
        for seq in 2..=6 {
            assert_eq!(
                see(&mut s, 1, 7, seq, t0 + Duration::from_millis(10)),
                Verdict::Deliver(seq),
                "seq={seq}"
            );
        }
        assert_eq!(s.last_seq(&1), Some(6));
    }

    // ── B1: what was held is not dropped, and the reported floor is true ──
    #[test]
    fn a_collapsed_window_delivers_what_it_held_and_names_that_first_number() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 1, 7, 5, t0);
        see(&mut s, 1, 7, 6, t0);
        let v = see(&mut s, 1, 7, 9, t0 + RESUME_TIMEOUT);
        assert_eq!(
            v,
            Verdict::Truncated {
                oldest_valid: 5,
                flushed: vec![5, 6, 9],
            },
            "★버린 뒤에 늦은 번호를 하한이라 신고하면 안 된다★"
        );
        assert_eq!(s.last_seq(&1), Some(9));
    }

    #[test]
    fn the_reported_lower_bound_is_a_number_we_actually_deliver() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 1, 7, 5, t0);
        let Verdict::Truncated {
            oldest_valid,
            flushed,
        } = see(&mut s, 1, 7, 8, t0 + RESUME_TIMEOUT)
        else {
            panic!("창이 닫혀야 한다");
        };
        assert_eq!(flushed.first().copied(), Some(oldest_valid));
    }

    #[test]
    fn a_full_hold_buffer_collapses_the_window_without_waiting() {
        let mut s: Streams<u8, u64> = Streams::new(1024, 3, RESUME_TIMEOUT);
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 1, 7, 5, t0);
        see(&mut s, 1, 7, 6, t0);
        let v = see(&mut s, 1, 7, 7, t0);
        assert_eq!(
            v,
            Verdict::Truncated {
                oldest_valid: 5,
                flushed: vec![5, 6, 7],
            },
            "붙드는 양에 상한이 없으면 상대가 우리 메모리를 민다"
        );
    }

    // ── B2: the deadline runs even without arrivals ──
    #[test]
    fn a_silent_peer_still_has_its_resume_window_closed() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 1, 7, 5, t0);
        assert!(s.expire_resumes(t0 + Duration::from_millis(1)).is_empty());

        let collapsed = s.expire_resumes(t0 + RESUME_TIMEOUT);
        assert_eq!(
            collapsed,
            vec![Truncation {
                key: 1,
                generation: 7,
                oldest_valid: 5,
                flushed: vec![5],
            }],
            "★조각이 더 안 오면 아무도 이 창을 못 닫는다 — 그게 평범한 경우다★"
        );
        assert!(s.pending_resumes().is_empty());
        assert_eq!(s.last_seq(&1), Some(5));
    }

    #[test]
    fn the_resume_deadline_is_visible_so_the_loop_can_wake_on_it() {
        let mut s = streams();
        let t0 = Instant::now();
        assert_eq!(s.next_resume_deadline(), None);
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 1, 7, 5, t0);
        assert_eq!(s.next_resume_deadline(), Some(t0 + RESUME_TIMEOUT));
        s.expire_resumes(t0 + RESUME_TIMEOUT);
        assert_eq!(s.next_resume_deadline(), None);
    }

    #[test]
    fn the_earliest_of_several_resume_deadlines_wins() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 2, 7, 1, t0);
        see(&mut s, 2, 7, 9, t0 + Duration::from_millis(50));
        see(&mut s, 1, 7, 9, t0 + Duration::from_millis(100));
        assert_eq!(
            s.next_resume_deadline(),
            Some(t0 + Duration::from_millis(50) + RESUME_TIMEOUT)
        );
    }

    #[test]
    fn a_gap_after_truncation_asks_once_more() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 1, 7, 5, t0);
        see(&mut s, 1, 7, 9, t0 + RESUME_TIMEOUT);
        assert!(matches!(
            see(&mut s, 1, 7, 20, t0 + RESUME_TIMEOUT),
            Verdict::Gap { .. }
        ));
    }

    #[test]
    fn only_one_resume_goes_out_per_gap() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        let asks = (5..12)
            .filter(|seq| matches!(see(&mut s, 1, 7, *seq, t0), Verdict::Gap { .. }))
            .count();
        assert_eq!(asks, 1);
    }

    #[test]
    fn an_outstanding_resume_survives_so_it_can_be_sent_again() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 1, 7, 5, t0);
        assert_eq!(s.pending_resumes(), vec![(1u8, 1u64)]);

        s.rearm_resumes(t0 + Duration::from_secs(60));
        assert_eq!(
            s.next_resume_deadline(),
            Some(t0 + Duration::from_secs(60) + RESUME_TIMEOUT)
        );
        assert!(s.expire_resumes(t0 + Duration::from_secs(60)).is_empty());
    }

    // ── 3-0 U1c: the caller's limit · a stopped clock · fixing a held piece ──

    #[test]
    fn a_chunk_past_the_callers_cap_closes_the_open_window_like_a_full_buffer() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 1, 7, 5, t0);
        assert_eq!(
            s.observe_capped(&mark(1, 7, 6), 6, t0, true),
            Verdict::Truncated {
                oldest_valid: 5,
                flushed: vec![5, 6],
            },
            "★개수 상한이 찬 것과 같은 길이다★ — 붙든 것을 순서대로 내보내고 그 첫 번호를 하한으로 올린다"
        );
        assert!(s.pending_resumes().is_empty());
        assert_eq!(s.last_seq(&1), Some(6));
    }

    #[test]
    fn a_gap_past_the_callers_cap_opens_no_window() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        assert_eq!(
            s.observe_capped(&mark(1, 7, 5), 5, t0, true),
            Verdict::Truncated {
                oldest_valid: 5,
                flushed: vec![5],
            },
            "★붙들 자리가 없으면 재요청 없이 이 조각부터 이어 간다★ — 그 앞은 잘림으로 신고된다"
        );
        assert!(s.pending_resumes().is_empty(), "창을 열지 않았다");
        assert_eq!(see(&mut s, 1, 7, 6, t0), Verdict::Deliver(6));
    }

    #[test]
    fn shifting_the_resume_clock_pushes_the_deadline_back() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 1, 7, 5, t0);
        let paused = Duration::from_secs(30);
        s.shift_resumes(paused);
        assert_eq!(s.next_resume_deadline(), Some(t0 + paused + RESUME_TIMEOUT));
        assert!(
            s.expire_resumes(t0 + RESUME_TIMEOUT).is_empty(),
            "★민 만큼 늦게 닫힌다★ — 그동안 답을 못 읽은 것은 상대 탓이 아니다"
        );
        assert_eq!(s.expire_resumes(t0 + paused + RESUME_TIMEOUT).len(), 1);
    }

    #[test]
    fn a_held_chunk_can_be_reached_until_its_window_closes() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 1, 7, 5, t0);
        *s.held_mut(&1, 5).expect("붙들고 있다") = 50;
        assert_eq!(s.held_mut(&1, 6), None, "안 붙든 번호");
        assert_eq!(s.held_mut(&2, 5), None, "모르는 스트림");
        let collapsed = s.expire_resumes(t0 + RESUME_TIMEOUT);
        assert_eq!(collapsed[0].flushed, vec![50], "고친 것이 그대로 나간다");
        assert_eq!(s.held_mut(&1, 5), None, "창이 닫히면 없다");
    }

    // ── When the window closes, progress goes to the last number of what was held ──
    //
    // ★There is no guarantee that the piece that made the window close is the largest number among those held★ — if progress is set to that piece's number, a number already sent out
    //   comes again as replay and is delivered once more against the order. The rule is the same on every closing path (byte limit · count limit · deadline ·
    //   teardown).

    /// After the window closes, the peer sends the rest of the replay (`replay`) — all the numbers that actually go up (including what was sent out at closing).
    fn delivered_after(
        s: &mut Streams<u8, u64>,
        flushed: Vec<u64>,
        replay: impl IntoIterator<Item = u64>,
        now: Instant,
    ) -> Vec<u64> {
        let mut out = flushed;
        for seq in replay {
            out.extend(see(s, 1, 7, seq, now).into_delivered());
        }
        out
    }

    #[test]
    fn a_descending_arrival_past_the_byte_cap_moves_the_cursor_to_the_highest_held() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        assert!(matches!(see(&mut s, 1, 7, 5, t0), Verdict::Gap { .. }));
        let Verdict::Truncated {
            oldest_valid,
            flushed,
        } = s.observe_capped(&mark(1, 7, 3), 3, t0, true)
        else {
            panic!("상한을 넘겨 창이 닫혀야 한다");
        };
        assert_eq!((oldest_valid, flushed.clone()), (3, vec![3, 5]));
        assert_eq!(
            delivered_after(&mut s, flushed, [2, 3, 4, 5, 6], t0),
            vec![3, 5, 6],
            "★늘 오름차순 · 겹침 없음★ — 진도를 3 에 두면 재생분 4 · 5 가 5 뒤에 다시 올라간다"
        );
    }

    #[test]
    fn a_partial_replay_past_the_byte_cap_moves_the_cursor_to_the_highest_held() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        assert!(matches!(see(&mut s, 1, 7, 4, t0), Verdict::Gap { .. }));
        assert!(matches!(see(&mut s, 1, 7, 5, t0), Verdict::Held { .. }));
        let Verdict::Truncated {
            oldest_valid,
            flushed,
        } = s.observe_capped(&mark(1, 7, 3), 3, t0, true)
        else {
            panic!("상한을 넘겨 창이 닫혀야 한다");
        };
        assert_eq!((oldest_valid, flushed.clone()), (3, vec![3, 4, 5]));
        assert_eq!(
            delivered_after(&mut s, flushed, [4, 5, 6], t0),
            vec![3, 4, 5, 6]
        );
    }

    #[test]
    fn a_descending_arrival_filling_the_hold_count_moves_the_cursor_to_the_highest_held() {
        let mut s: Streams<u8, u64> = Streams::new(1024, 2, RESUME_TIMEOUT);
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 1, 7, 5, t0);
        let Verdict::Truncated { flushed, .. } = see(&mut s, 1, 7, 3, t0) else {
            panic!("개수 상한이 차 창이 닫혀야 한다");
        };
        assert_eq!(
            delivered_after(&mut s, flushed, [4, 5, 6], t0),
            vec![3, 5, 6]
        );
    }

    #[test]
    fn an_expired_window_with_scattered_numbers_moves_the_cursor_to_the_highest_held() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 1, 7, 5, t0);
        assert!(matches!(see(&mut s, 1, 7, 3, t0), Verdict::Held { .. }));
        let mut collapsed = s.expire_resumes(t0 + RESUME_TIMEOUT);
        let flushed = collapsed.remove(0).flushed;
        assert_eq!(
            delivered_after(&mut s, flushed, [4, 5, 6], t0 + RESUME_TIMEOUT),
            vec![3, 5, 6],
            "★첫 번호 + 붙든 수 − 1 은 끝 번호가 아니다★ — {{3, 5}} 면 4 가 되어 5 를 한 번 더 올린다"
        );
    }

    #[test]
    fn closing_every_window_hands_back_what_each_held() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 2, 7, 1, t0);
        see(&mut s, 3, 7, 1, t0);
        see(&mut s, 1, 7, 4, t0);
        see(&mut s, 1, 7, 6, t0);
        see(&mut s, 3, 7, 9, t0);
        let closed: Vec<(u8, u64, Vec<u64>)> = s
            .close_resumes()
            .into_iter()
            .map(|t| (t.key, t.oldest_valid, t.flushed))
            .collect();
        assert_eq!(
            closed,
            vec![(1, 4, vec![4, 6]), (3, 9, vec![9])],
            "★시한 전이어도 열린 창은 다 접힌다★ — 창이 없는 스트림은 건너뛴다"
        );
        assert!(s.pending_resumes().is_empty());
        assert_eq!(s.last_seq(&1), Some(6));
    }

    #[test]
    fn an_answered_resume_stops_being_pending_and_drops_what_it_held() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 1, 7, 5, t0);
        assert_eq!(see(&mut s, 1, 7, 2, t0), Verdict::Deliver(2));
        assert!(s.pending_resumes().is_empty());
        // The 5 that was held is carried again by the peer's replay — if sent out here, it is delivered twice.
        assert_eq!(see(&mut s, 1, 7, 3, t0), Verdict::Deliver(3));
    }

    #[test]
    fn generation_change_resets_the_seq_ledger_and_says_what_it_dropped() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 1, 7, 5, t0);
        see(&mut s, 1, 7, 6, t0);
        assert_eq!(
            see(&mut s, 1, 8, 1, t0),
            Verdict::GenerationChanged {
                from: 7,
                to: 8,
                msg: 1,
                discarded: 2,
            }
        );
        assert!(s.pending_resumes().is_empty());
        assert_eq!(s.last_seq(&1), Some(1));
    }

    #[test]
    fn generation_is_compared_for_equality_only_never_for_order() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 9, 50, t0);
        assert!(matches!(
            see(&mut s, 1, 2, 1, t0),
            Verdict::GenerationChanged { from: 9, to: 2, .. }
        ));
    }

    #[test]
    fn streams_do_not_share_state() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 2, 7, 100, t0);
        assert_eq!(see(&mut s, 1, 7, 2, t0), Verdict::Deliver(2));
        assert_eq!(see(&mut s, 2, 7, 101, t0), Verdict::Deliver(101));
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn forget_drops_one_stream_only() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 2, 7, 1, t0);
        s.forget(&1);
        assert_eq!(s.last_seq(&1), None);
        assert_eq!(s.last_seq(&2), Some(1));
    }

    // ── N5/B3: limit and eviction ──
    #[test]
    fn a_peer_inventing_stream_keys_cannot_grow_the_map_without_bound() {
        let mut s: Streams<u32, u64> = Streams::new(8, 256, RESUME_TIMEOUT);
        let t0 = Instant::now();
        for key in 0..10_000u32 {
            s.observe(
                &StreamMark {
                    key,
                    generation: 1,
                    seq: 1,
                },
                1,
                t0,
            );
            s.drain_evicted();
        }
        assert_eq!(s.len(), 8);
    }

    #[test]
    fn eviction_takes_the_least_recently_used_not_the_oldest_registration() {
        let mut s: Streams<u8, u64> = Streams::new(2, 256, RESUME_TIMEOUT);
        let t0 = Instant::now();
        see(&mut s, 1, 1, 1, t0);
        see(&mut s, 2, 1, 1, t0);
        // Keep using 1 — the most active stream must not become the first victim.
        see(&mut s, 1, 1, 2, t0);
        see(&mut s, 3, 1, 1, t0);
        assert_eq!(s.last_seq(&1), Some(2), "★가장 활발한 스트림이 살아남는다★");
        assert_eq!(s.last_seq(&2), None);
        assert_eq!(s.last_seq(&3), Some(1));
    }

    #[test]
    fn eviction_never_swallows_an_outstanding_resume_silently() {
        let mut s: Streams<u8, u64> = Streams::new(1, 256, RESUME_TIMEOUT);
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 1, 7, 5, t0);
        assert_eq!(s.pending_resumes().len(), 1);

        see(&mut s, 2, 7, 1, t0);
        let evicted = s.drain_evicted();
        assert_eq!(
            evicted,
            vec![Evicted {
                key: 1,
                generation: 7,
                last_seq: 1,
                flushed: vec![5],
                oldest_valid: Some(5),
            }],
            "★붙들고 있던 조각이 축출과 함께 조용히 사라지면 안 된다★"
        );
    }

    #[test]
    fn evicting_a_quiet_track_reports_it_with_nothing_held() {
        let mut s: Streams<u8, u64> = Streams::new(1, 256, RESUME_TIMEOUT);
        let t0 = Instant::now();
        see(&mut s, 1, 7, 3, t0);
        see(&mut s, 2, 7, 1, t0);
        assert_eq!(
            s.drain_evicted(),
            vec![Evicted {
                key: 1,
                generation: 7,
                last_seq: 3,
                flushed: vec![],
                oldest_valid: None,
            }]
        );
    }

    #[test]
    fn forget_keeps_the_eviction_order_consistent() {
        let mut s: Streams<u8, u64> = Streams::new(2, 256, RESUME_TIMEOUT);
        let t0 = Instant::now();
        see(&mut s, 1, 1, 1, t0);
        see(&mut s, 2, 1, 1, t0);
        s.forget(&1);
        see(&mut s, 3, 1, 1, t0);
        assert_eq!(s.len(), 2);
        assert_eq!(s.last_seq(&2), Some(1));
        assert_eq!(s.last_seq(&3), Some(1));
    }
}
