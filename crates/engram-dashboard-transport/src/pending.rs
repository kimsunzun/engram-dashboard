//! Request/response correlation — ★pure layer★. The pending table · deadlines · rejecting reuse of expired numbers.
//!
//! ★This file has no tokio, no socket, and no workspace crate★ — the gate measures that (canonical =
//! the `lib.rs` header). The waking handle (something like `oneshot::Sender`) is only held as a **generic slot `S`**,
//! and what actually wakes it is the supervisor task of [`crate::peer`].
//!
//! ★The supervisor task owns the table alone (no lock)★.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::hash::Hash;
use std::time::{Duration, Instant};

use crate::machine::{DisconnectCause, Generation};

/// All the ways a request fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestError {
    /// The deadline passed. ★No automatic resend★ — no response is not "failure" but **"unknown"**, so if sent again
    /// an operation the peer had actually processed is applied twice (ADR-0181 decision 3).
    TimedOut { after: Duration },
    /// Ended because it was disconnected — either before sending or after sending and before the reply. Which one is told by `sent` (TRD 3 D12).
    Disconnected {
        /// The connection this request is bound to — the generation the caller captured ([`crate::Peer::request_in`] · `request` uses the current
        /// generation at the moment of the call). For a sent request it is the connection it was sent on.
        generation: Generation,
        /// The last known disconnect cause. ★It may not be why `generation` ended★ — if it disconnected again after that, it is the later one.
        cause: DisconnectCause,
        /// ★May it have reached the peer★. `false` = it ended before being handed to the channel (not in the operating phase · disconnected while in the queue ·
        /// the bound generation was not the current one) — it did not reach the peer, so sending again is not applied twice. `true` = it disconnected after
        /// getting into the pending table (it gets in just before the write) — the peer may have received and processed it so we do not know the result (no blind retry —
        /// the same position as [`RequestError::TimedOut`]).
        /// ★`true` can be inflated and `false` is a guarantee★ — it is `true` even if it disconnected with the write never polled after getting into the table,
        /// and even when the supervisor vanished abnormally (panic · runtime shutdown) and we do not know how far it got.
        sent: bool,
    },
    /// The outbound queue is full.
    QueueFull,
    /// An old request with the same number is still waiting for its answer and a new request reused that number — the old waiter wakes with this.
    /// ★The new request does not inherit the slot and is rejected as [`RequestError::TagRetired`] (both fail)★ — the correlation key is that number
    /// alone, so the late reply of the request that went out first cannot be told apart from the new request's counterpart (`lib.rs` 「알려진 한계」 (known limits) ·
    /// [`Insert::Collided`]).
    Superseded,
    /// This command takes no reply ([`crate::Wire::request_tag`] is `None`). It means use `notify`.
    NotARequest,
    /// It reused a retired number — either a number retired by deadline expiry, or a new request that overlapped a waiting number (that number is also
    /// retired then — the counterpart of [`RequestError::Superseded`]).
    TagRetired,
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TimedOut { after } => write!(f, "no reply within {after:?}"),
            Self::Disconnected {
                generation,
                cause,
                sent,
            } => {
                let sent = if *sent {
                    "may have been sent"
                } else {
                    "not sent"
                };
                write!(
                    f,
                    "disconnected (generation {}, {cause:?}, {sent})",
                    generation.0
                )
            }
            Self::QueueFull => f.write_str("outbound queue full"),
            Self::Superseded => f.write_str("request id reused by a newer request"),
            Self::NotARequest => f.write_str("this message does not take a reply"),
            Self::TagRetired => {
                f.write_str("request id is retired (an earlier timeout or a collision)")
            }
        }
    }
}

impl std::error::Error for RequestError {}

/// A bounded ring that remembers expired numbers.
///
/// ★The real guarantee is not here but in **the side that makes the numbers**★ — in our case a uuid v4. All this ring can do
/// is reject when a **recent** expired number comes back, and memory cannot be infinite. When it overflows it quietly forgets from the oldest,
/// and that number becomes reusable. ★Do not read this limit as a guarantee★.
#[derive(Debug)]
struct RetiredRing<T> {
    order: VecDeque<T>,
    set: HashSet<T>,
    cap: usize,
}

impl<T: Eq + Hash + Clone> RetiredRing<T> {
    fn new(cap: usize) -> Self {
        Self {
            order: VecDeque::new(),
            set: HashSet::new(),
            cap,
        }
    }

    fn insert(&mut self, tag: T) {
        if self.cap == 0 || !self.set.insert(tag.clone()) {
            return;
        }
        self.order.push_back(tag);
        while self.order.len() > self.cap {
            if let Some(old) = self.order.pop_front() {
                self.set.remove(&old);
            }
        }
    }

    fn contains(&self, tag: &T) -> bool {
        self.set.contains(tag)
    }

    fn len(&self) -> usize {
        self.order.len()
    }
}

/// The result of [`Pending::insert`]. On any branch it **does not lose the slot** — a slot that could not be put in is returned as is, so
/// the caller must wake it.
#[derive(Debug)]
pub enum Insert<S> {
    /// Took a place.
    Placed,
    /// The same number was already **waiting** — retire that number and wake both.
    ///
    /// The old waiter gets [`RequestError::Superseded`] (preserving today's behavior), the new request gets
    /// [`RequestError::TagRetired`]. Why we must not let the new one inherit it: the correlation key is this number
    /// alone, so **the late reply of the request that went out first would be delivered as the counterpart of the later request.** There is nothing on the wire to
    /// tell which one a reply belongs to, so rather than deliver wrongly we fail both.
    Collided { displaced: S, rejected: S },
    /// Reused a number that had expired.
    Retired(S),
}

#[derive(Debug)]
struct Waiter<S> {
    slot: S,
    deadline: Instant,
}

/// Pairing number → pending slot.
///
/// ## ★Known limit — it cannot **make misdelivery of a late reply impossible**★ (do not try to fix)
///
/// If the consumer picks a type that **reuses** `Tag`, this crate cannot prevent a late reply from being delivered to the wrong request,
/// and **does not try to.** The mechanism and its limit are as follows:
///
/// - A number collected by deadline expiry is put into a **bounded ring** by [`Pending`], and if it comes again it is rejected with
///   [`RequestError::TagRetired`]. Ring length = [`crate::Policy::retired_tags`] (default 256).
/// - The ring is **FIFO**. If 256 more retire after that, the oldest number is **forgotten**.
/// - ★If a late reply for that number arrives after it is forgotten, it is delivered as the counterpart of a **new request** that used the same number.★
///   The correlation key is that number alone, so there is nothing on the wire to tell the two replies apart.
///
/// ★**Forgetting is not a defect but half of what this ring does**★ — if it did not forget, **legitimately**
/// reusing that number would be blocked forever too. That is, 「오배달 완전 차단」 (blocking misdelivery completely) and 「정당한 재사용 허용」 (allowing legitimate reuse) are the two
/// ends of the same knob, and unbounded memory is impossible from the start (it grows without limit).
///
/// ★The two reviewers split here and the split itself is the record★ — one read it as 「누수 없음, 잊는 것이 정당한
/// 재사용을 가능케 한다」 (no leak; forgetting makes legitimate reuse possible), the other as 「그 잊음이 원래 답장을 새 대기자에게 보낸다」 (that forgetting sends the original reply to a new waiter). Both are
/// right, and **leaving the bounded ring exactly as it is now is the user's decision** (2026-09-06). Basis: our real
/// `Tag` is a UUID so **a legitimate collision never occurs in the first place**. The remaining hole needs ★a consumer bug (reusing numbers)★ and
/// ★a reply that arrives only after more retirements than fill one ring (default 256 times)★ **at the same time**.
/// The regression net = `the_retired_ring_forgets_its_oldest_when_it_overflows` (it pins that forgetting **as intended**).
#[derive(Debug)]
pub struct Pending<T, S> {
    waiting: HashMap<T, Waiter<S>>,
    retired: RetiredRing<T>,
}

impl<T: Eq + Hash + Clone, S> Pending<T, S> {
    pub fn new(retired_cap: usize) -> Self {
        Self {
            waiting: HashMap::new(),
            retired: RetiredRing::new(retired_cap),
        }
    }

    /// Registers a slot. ★Call it **before sending**★ — reverse it and a fast reply cannot find its slot.
    pub fn insert(&mut self, tag: T, slot: S, deadline: Instant) -> Insert<S> {
        if self.retired.contains(&tag) {
            return Insert::Retired(slot);
        }
        if let Some(old) = self.waiting.remove(&tag) {
            self.retired.insert(tag);
            return Insert::Collided {
                displaced: old.slot,
                rejected: slot,
            };
        }
        self.waiting.insert(tag, Waiter { slot, deadline });
        Insert::Placed
    }

    /// A reply came. If there is no slot, `None` — ★a reply that arrives late after expiry is quietly dropped that way★.
    pub fn take(&mut self, tag: &T) -> Option<S> {
        self.waiting.remove(tag).map(|w| w.slot)
    }

    /// Collects those whose deadline has passed. A collected number is registered in the ring and later reuse is rejected.
    pub fn expire(&mut self, now: Instant) -> Vec<(T, S)> {
        let due: Vec<T> = self
            .waiting
            .iter()
            .filter(|(_, w)| w.deadline <= now)
            .map(|(t, _)| t.clone())
            .collect();
        due.into_iter()
            .filter_map(|tag| {
                let waiter = self.waiting.remove(&tag)?;
                self.retired.insert(tag.clone());
                Some((tag, waiter.slot))
            })
            .collect()
    }

    /// Disconnected — collects all waiters. ★This side does not register in the ring★: it is not that the number died but that the connection
    /// died, so it is legitimate for the consumer to request again with the same number.
    pub fn drain(&mut self) -> Vec<(T, S)> {
        self.waiting.drain().map(|(t, w)| (t, w.slot)).collect()
    }

    /// The earliest deadline. `None` means nothing is waiting.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.waiting.values().map(|w| w.deadline).min()
    }

    pub fn contains(&self, tag: &T) -> bool {
        self.waiting.contains_key(tag)
    }

    pub fn is_retired(&self, tag: &T) -> bool {
        self.retired.contains(tag)
    }

    pub fn len(&self) -> usize {
        self.waiting.len()
    }

    pub fn is_empty(&self) -> bool {
        self.waiting.is_empty()
    }

    pub fn retired_len(&self) -> usize {
        self.retired.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> Pending<u32, &'static str> {
        Pending::new(4)
    }

    #[test]
    fn a_reply_wakes_exactly_its_own_slot() {
        let mut p = table();
        let t0 = Instant::now();
        p.insert(1, "a", t0 + Duration::from_secs(8));
        p.insert(2, "b", t0 + Duration::from_secs(8));
        assert_eq!(p.take(&1), Some("a"));
        assert_eq!(p.take(&1), None);
        assert_eq!(p.len(), 1);
        assert!(p.contains(&2));
    }

    #[test]
    fn only_the_expired_slot_is_woken() {
        let mut p = table();
        let t0 = Instant::now();
        p.insert(1, "early", t0 + Duration::from_secs(8));
        p.insert(2, "late", t0 + Duration::from_secs(30));

        let fired = p.expire(t0 + Duration::from_secs(8));
        assert_eq!(fired, vec![(1, "early")]);
        assert_eq!(p.len(), 1);
        assert!(p.contains(&2));
    }

    #[test]
    fn nothing_expires_before_its_deadline() {
        let mut p = table();
        let t0 = Instant::now();
        p.insert(1, "a", t0 + Duration::from_secs(8));
        assert!(p.expire(t0 + Duration::from_millis(7999)).is_empty());
        assert_eq!(p.len(), 1);
    }

    #[test]
    fn a_reply_that_arrives_after_the_timeout_finds_no_slot() {
        let mut p = table();
        let t0 = Instant::now();
        p.insert(7, "a", t0 + Duration::from_secs(8));
        p.expire(t0 + Duration::from_secs(8));
        assert_eq!(p.take(&7), None);
    }

    #[test]
    fn an_expired_tag_may_not_be_used_again() {
        let mut p = table();
        let t0 = Instant::now();
        p.insert(7, "a", t0 + Duration::from_secs(8));
        p.expire(t0 + Duration::from_secs(8));
        assert!(p.is_retired(&7));

        match p.insert(7, "b", t0 + Duration::from_secs(20)) {
            Insert::Retired(slot) => assert_eq!(slot, "b", "슬롯을 돌려줘야 호출자가 깨울 수 있다"),
            other => panic!("만료된 번호가 그대로 다시 들어갔다: {other:?}"),
        }
        assert!(p.is_empty());
    }

    // ── M1: there is only one correlation key, so an overlapping number cannot tell deliveries apart ──
    #[test]
    fn a_colliding_tag_wakes_both_sides_and_retires_the_number() {
        let mut p = table();
        let t0 = Instant::now();
        p.insert(9, "old", t0 + Duration::from_secs(8));
        match p.insert(9, "new", t0 + Duration::from_secs(8)) {
            Insert::Collided {
                displaced,
                rejected,
            } => {
                assert_eq!(displaced, "old");
                assert_eq!(rejected, "new");
            }
            other => panic!("{other:?}"),
        }
        assert!(p.is_empty(), "어느 쪽도 표에 남지 않는다");
        assert!(p.is_retired(&9));
    }

    #[test]
    fn a_reply_for_a_collided_tag_reaches_nobody() {
        let mut p = table();
        let t0 = Instant::now();
        p.insert(9, "first", t0 + Duration::from_secs(8));
        p.insert(9, "second", t0 + Duration::from_secs(8));
        assert_eq!(
            p.take(&9),
            None,
            "먼저 나간 요청의 늦은 답장이 나중 요청의 짝으로 배달되면 안 된다"
        );
    }

    #[test]
    fn a_disconnect_takes_everyone_and_retires_nobody() {
        let mut p = table();
        let t0 = Instant::now();
        p.insert(1, "a", t0 + Duration::from_secs(8));
        p.insert(2, "b", t0 + Duration::from_secs(8));

        let mut drained = p.drain();
        drained.sort();
        assert_eq!(drained, vec![(1, "a"), (2, "b")]);
        assert!(p.is_empty());
        assert!(!p.is_retired(&1));
        assert!(matches!(
            p.insert(1, "again", t0 + Duration::from_secs(8)),
            Insert::Placed
        ));
    }

    #[test]
    fn the_retired_ring_forgets_its_oldest_when_it_overflows() {
        let mut p: Pending<u32, &'static str> = Pending::new(2);
        let t0 = Instant::now();
        for tag in 1..=3u32 {
            p.insert(tag, "x", t0);
            p.expire(t0);
        }
        assert_eq!(p.retired_len(), 2);
        assert!(!p.is_retired(&1), "가장 오래된 것은 조용히 잊는다");
        assert!(p.is_retired(&2));
        assert!(p.is_retired(&3));
        assert!(
            matches!(
                p.insert(1, "reused", t0 + Duration::from_secs(8)),
                Insert::Placed
            ),
            "잊힌 번호는 다시 쓸 수 있게 된다 — 이 링은 보증이 아니다"
        );
    }

    #[test]
    fn a_zero_length_ring_retires_nothing() {
        let mut p: Pending<u32, &'static str> = Pending::new(0);
        let t0 = Instant::now();
        p.insert(1, "x", t0);
        p.expire(t0);
        assert!(!p.is_retired(&1));
        assert_eq!(p.retired_len(), 0);
    }

    #[test]
    fn next_deadline_is_the_earliest_one() {
        let mut p = table();
        let t0 = Instant::now();
        assert_eq!(p.next_deadline(), None);
        p.insert(1, "a", t0 + Duration::from_secs(30));
        p.insert(2, "b", t0 + Duration::from_secs(8));
        assert_eq!(p.next_deadline(), Some(t0 + Duration::from_secs(8)));
        p.take(&2);
        assert_eq!(p.next_deadline(), Some(t0 + Duration::from_secs(30)));
    }

    #[test]
    fn request_errors_say_what_happened() {
        assert!(RequestError::TimedOut {
            after: Duration::from_secs(8)
        }
        .to_string()
        .contains('8'));
        assert!(RequestError::TagRetired.to_string().contains("retired"));
        assert!(RequestError::NotARequest.to_string().contains("reply"));
        assert!(RequestError::Disconnected {
            generation: Generation(3),
            cause: DisconnectCause::Idle,
            sent: false,
        }
        .to_string()
        .contains('3'));
        // ★Tell the two positions apart in the text too★ — the shell emits these two with different wording (safe to resend · result unknown) (TRD 3 D12).
        let text = |sent| {
            RequestError::Disconnected {
                generation: Generation(3),
                cause: DisconnectCause::Idle,
                sent,
            }
            .to_string()
        };
        assert!(text(false).contains("not sent"));
        assert!(text(true).contains("may have been sent"));
    }
}
