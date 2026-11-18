//! Request/response correlation — ★pure layer★: the pending table · deadlines · rejecting reuse of expired numbers. ★No tokio, socket or workspace crate★ (gate: `lib.rs` header); the waking handle is only
//! a **generic slot `S`**, woken by the [`crate::peer`] supervisor. ★The supervisor owns the table alone (no lock)★.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::hash::Hash;
use std::time::{Duration, Instant};

use crate::machine::{DisconnectCause, Generation};

/// All the ways a request fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestError {
    /// The deadline passed. ★No automatic resend★ — no response is "unknown", not "failed"; resending could apply an operation the peer already processed twice (ADR-0181 decision 3).
    TimedOut {
        after: Duration,
    },
    /// Ended because it was disconnected, before sending or after sending and before the reply; `sent` says which (TRD 3 D12).
    Disconnected {
        /// The connection this request is bound to — the generation the caller captured ([`crate::Peer::request_in`]; `request` uses the current one at the call); for a sent request, the one it was sent on.
        generation: Generation,
        /// The last known disconnect cause. ★It may not be why `generation` ended★ — a later disconnect's if it disconnected again.
        cause: DisconnectCause,
        /// ★May it have reached the peer★. `false` = ended before being handed to the channel (not operating · disconnected while queued · bound generation not current) — it did not reach the peer, so
        /// resending is not applied twice. `true` = disconnected after entering the pending table (just before the write) — the peer may have processed it, result unknown (no blind retry; same position as
        /// [`RequestError::TimedOut`]). ★`true` can be inflated, `false` is a guarantee★ — `true` even if the write was never polled after entering the table, or the supervisor vanished abnormally.
        sent: bool,
    },
    QueueFull,
    /// An older request with the same number still waits and a new one reused it — the old waiter wakes with this. ★The new request does not inherit the slot and is rejected as
    /// [`RequestError::TagRetired`] (both fail)★ — the number is the only key, so the first request's late reply cannot be told from the new one's (`lib.rs` 「known limits」 · [`Insert::Collided`]).
    Superseded,
    /// This command takes no reply ([`crate::Wire::request_tag`] is `None`) — use `notify`.
    NotARequest,
    /// It reused a retired number — one retired by deadline expiry, or a new request overlapping a waiting number (also retired then; the counterpart of [`RequestError::Superseded`]).
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

/// A bounded ring that remembers expired numbers. ★The real guarantee is the side that **makes the numbers**★ (ours: uuid v4); this ring only rejects a **recent** expired number coming back, memory
/// being finite — on overflow it quietly forgets the oldest, which becomes reusable. ★Not a guarantee★.
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

/// The result of [`Pending::insert`]. ★No branch loses the slot★ — a slot that could not be put in is returned, and the caller must wake it.
#[derive(Debug)]
pub enum Insert<S> {
    Placed,
    /// The same number was already **waiting** — retire it and wake both: the old waiter gets [`RequestError::Superseded`] (today's behavior), the new request [`RequestError::TagRetired`]. The new one must
    /// not inherit it: the number is the only key, so **the first request's late reply would be delivered as the later request's counterpart**; nothing on the wire tells them apart, so both fail.
    Collided {
        displaced: S,
        rejected: S,
    },
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
/// If the consumer picks a type that **reuses** `Tag`, this crate cannot prevent a late reply reaching the wrong request, and does not try. A number collected by deadline expiry goes into a **FIFO bounded ring**
/// ([`crate::Policy::retired_tags`], default 256) and is rejected with [`RequestError::TagRetired`] if it returns; after 256 more retire, the oldest is **forgotten**, and ★a late reply for it is then delivered as
/// the counterpart of a **new request** using the same number★ (the number is the only key). ★Forgetting is half of what the ring does★ — without it a **legitimate** reuse is blocked forever and memory is
/// unbounded: 「block misdelivery completely」 and 「allow legitimate reuse」 are two ends of one knob.
///
/// ★Two reviewers split here; keeping the bounded ring is the user's decision★: our real `Tag` is a UUID so a legitimate collision never occurs; the remaining hole needs ★a consumer bug (reusing numbers)★ and
/// ★a reply arriving only after more retirements than one ring (default 256)★ together. Regression net: `the_retired_ring_forgets_its_oldest_when_it_overflows` (pins the forgetting **as intended**).
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

    /// Registers a slot. ★Call it **before sending**★ — reversed, a fast reply cannot find its slot.
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

    pub fn take(&mut self, tag: &T) -> Option<S> {
        self.waiting.remove(tag).map(|w| w.slot)
    }

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

    /// Disconnected — collects all waiters. ★Does not register in the ring★: the connection died, not the number, so requesting again with the same number is legitimate.
    pub fn drain(&mut self) -> Vec<(T, S)> {
        self.waiting.drain().map(|(t, w)| (t, w.slot)).collect()
    }

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
            Insert::Retired(slot) => assert_eq!(
                slot, "b",
                "the slot must be returned for the caller to be able to wake it"
            ),
            other => panic!("an expired number went in again as is: {other:?}"),
        }
        assert!(p.is_empty());
    }

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
        assert!(p.is_empty(), "neither stays in the table");
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
            "a late reply to an earlier request must not be delivered as the pair of a later request"
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
        assert!(!p.is_retired(&1), "the oldest is silently forgotten");
        assert!(p.is_retired(&2));
        assert!(p.is_retired(&3));
        assert!(
            matches!(
                p.insert(1, "reused", t0 + Duration::from_secs(8)),
                Insert::Placed
            ),
            "a forgotten number becomes usable again — this ring is not a guarantee"
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
