//! Byte tally of what has been received — ★driving layer★ (TRD 3 §3-0 ① · ADR-0310 decision 17).
//!
//! One tally per supervisor ([`Ledger`]). When the read task reads a frame it adds the payload length ([`Ledger::charge`]); that share ([`Charge`]) travels with the item through the read-ahead queue →
//! ordering area → upward queue (possibly held by the resume window) and is subtracted when it drops — taken out by the receiver, or discarded by the supervisor (a reply · an overlapping fragment · a fragment
//! the window drops when the answer arrives). ★One tally covers the three layers and the window★ — per-layer caps would let each fill to its own cap and the sum grow as a product (512 read-ahead entries ×
//! the one-message cap). ★The share rides on the item, not a layer★: it holds the tally of the supervisor that put it in while crossing read-ahead queue · ordering area · inbox ([`crate::Inbox`]); dropping the
//! inbox drops the shares with the items.
//!
//! ★The window share is counted separately inside the tally★ — bytes of stream fragments the resume window holds; added on holding ([`Charge::hold`]), subtracted on release or discard; the fill level is
//! unchanged. Budget B splits into a read share and a window cap.
//! - Read gate — if 「fill level − window share」 ≥ the read share, the read task stops reading. It decides by 「the fill level before adding」 (as net `CONN_RX_MAX_BYTES`), so while open one frame gets in
//!   whatever its size. ★Subtracting the window share at the gate is the point★ — held fragments must not stop reading, or the replay that would fill their hole is unread and, arriving late after the deadline
//!   close, dropped as a past number: a fillable hole becomes a permanent loss.
//! - Window cap — the window share never exceeds the cap, even by one frame; a fragment that would is not held and the window closes as truncated (`Supervisor::route`).
//!
//! ★Splitting starts at the first stream fragment ([`Ledger::use_streams`])★: before, read share = B · window cap = 0; after, half and half ([`split`]). A non-stream peer (ADR-0310 decision 16 — both Wires'
//! `stream_mark` is always `None`) has nothing for the window, so splitting from the start would only halve the read gate. `Wire` has no announcing field (adding one breaks every consumer — `wire.rs`),
//! so it is learned from the first fragment.
//!
//! ★Ceiling of the whole tally = read share + window cap + one message = B + M★ (TRD 3 §3-0 ①): only a read grows the tally, and only while the gate is open (fill level < read share + window cap), one frame
//! at a time; window put/take does not change the fill level. It also holds when splitting starts (the read share only drops, so the gate only moves toward closing). The one-message cap is the adapter's
//! (crate header 「known limits」).
//!
//! ★One lock guards the tally★ — three tasks open/close the gate (read task: adds then closes · supervisor: window put/take/discard · receiver: takes out). Beside the gate state it records when the gate closed and
//! the closed-and-reopened stretches ([`Ledger::take_shut`]), so a supervisor noticing a hold late, or one that missed a close/reopen while waiting to write, does not lose that time.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use tokio::sync::Notify;

use crate::clock::Clock;

/// Splits the budget B of a stream-using supervisor into (read share, window cap). The read share is never 0 (the gate would never open). Half and half: the two must sum to B for the B + M ceiling
/// (ADR-0310 decision 17), and it is the simplest split that starves neither (an internal value, no measured basis).
pub(crate) const fn split(budget: usize) -> (usize, usize) {
    let read = if budget / 2 == 0 { 1 } else { budget / 2 };
    (read, budget.saturating_sub(read))
}

pub(crate) struct Ledger {
    counts: Mutex<Counts>,
    /// Budget B — the read share and window cap are derived by [`Ledger::caps`] depending on whether streams are used.
    budget: usize,
    /// Clock recording when the gate closed — must be the supervisor's own so it agrees with its deadline calculation.
    clock: Arc<dyn Clock>,
    /// The read gate opened — the read task's share.
    reader_room: Notify,
    /// The same fact, the supervisor's share. ★The wake-up handle is split in two★ — `notify_one` wakes one waiter and stores one permit, so a shared handle would let one side eat the other's permit.
    holder_room: Notify,
}

struct Counts {
    level: usize,
    held: usize,
    /// Whether a stream fragment has been seen — after that the budget is split in half ([`Ledger::use_streams`]).
    streams: bool,
    /// The time the gate closed, if the read gate is closed.
    shut_at: Option<Instant>,
    /// Stretches that closed and reopened since the last [`Ledger::take_shut`].
    closed: Closed,
}

/// Summary of the stretches where the read gate closed and reopened. Stretches do not overlap and are in time order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Closed {
    /// Sum of lengths.
    pub total: Duration,
    /// Time the first stretch closed. `None` if there are no stretches.
    pub from: Option<Instant>,
    /// Time the last stretch opened. `None` if there are no stretches.
    pub until: Option<Instant>,
}

impl Closed {
    fn add(&mut self, from: Instant, until: Instant) {
        self.total = self
            .total
            .saturating_add(until.saturating_duration_since(from));
        self.from.get_or_insert(from);
        self.until = Some(until);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Shut {
    /// The time it closed, if it is closed now.
    pub since: Option<Instant>,
    /// Stretches that closed and reopened in between — all earlier than `since`.
    pub closed: Closed,
}

impl Ledger {
    pub(crate) fn new(budget: usize, clock: Arc<dyn Clock>) -> Arc<Self> {
        Arc::new(Self {
            counts: Mutex::new(Counts {
                level: 0,
                held: 0,
                streams: false,
                shut_at: None,
                closed: Closed::default(),
            }),
            budget,
            clock,
            reader_room: Notify::new(),
            holder_room: Notify::new(),
        })
    }

    /// (read share, window cap) — before streams, the whole budget is the read share. The sum is at most the budget so B + M holds (budget 0 → read share 1, so the gate never stays shut forever).
    fn caps(&self, streams: bool) -> (usize, usize) {
        if streams {
            split(self.budget)
        } else {
            (self.budget.max(1), 0)
        }
    }

    // Inside the lock there is only add · subtract · read time, so the tally stays intact even when poisoned.
    fn counts(&self) -> MutexGuard<'_, Counts> {
        self.counts.lock().unwrap_or_else(PoisonError::into_inner)
    }

    #[cfg(test)]
    pub(crate) fn level(&self) -> usize {
        self.counts().level
    }

    /// The part of it held by the resume window.
    #[cfg(test)]
    pub(crate) fn held(&self) -> usize {
        self.counts().held
    }

    /// Whether the read gate is closed — the condition under which the read task does not read the link.
    pub(crate) fn over(&self) -> bool {
        self.counts().shut_at.is_some()
    }

    /// If the gate is closed now, returns when it closed and the closed-and-reopened stretches since the last call, and forgets those stretches (tally's clock). Used by `Supervisor::track_pause`
    /// — with two callers one would not see what the other took.
    pub(crate) fn take_shut(&self) -> Shut {
        let mut c = self.counts();
        Shut {
            since: c.shut_at,
            closed: std::mem::take(&mut c.closed),
        }
    }

    /// A stream fragment was seen — the budget is split from here (header); never undone. The read share only shrinks, so the gate may close but never opens because of this.
    pub(crate) fn use_streams(&self) {
        self.update(|c| c.streams = true);
    }

    /// Whether these bytes can still be held in the window. Unlike the read gate it measures 「after adding」 — never over the cap, even by one frame. Before [`Ledger::use_streams`] the cap is 0, so always false.
    pub(crate) fn window_fits(&self, bytes: usize) -> bool {
        let c = self.counts();
        c.held.saturating_add(bytes) <= self.caps(c.streams).1
    }

    pub(crate) fn charge(self: &Arc<Self>, bytes: usize) -> Charge {
        if bytes == 0 {
            return Charge::none();
        }
        self.update(|c| c.level += bytes);
        Charge {
            ledger: Some(self.clone()),
            bytes,
            held: false,
        }
    }

    pub(crate) async fn reader_room(&self) {
        while self.over() {
            self.reader_room.notified().await;
        }
    }

    /// The supervisor's share of the same wait. ★Cancel-safe — usable on a `select!` arm★ (checks the gate first on every poll).
    pub(crate) async fn holder_room(&self) {
        while self.over() {
            self.holder_room.notified().await;
        }
    }

    fn update(&self, change: impl FnOnce(&mut Counts)) {
        let opened = {
            let mut c = self.counts();
            change(&mut c);
            let shut = c.level - c.held >= self.caps(c.streams).0;
            match (c.shut_at, shut) {
                (None, true) => {
                    c.shut_at = Some(self.clock.now());
                    false
                }
                (Some(since), false) => {
                    c.shut_at = None;
                    c.closed.add(since, self.clock.now());
                    true
                }
                _ => false,
            }
        };
        // ★Wake on every gate opening★ — the woken side re-checks and sleeps again if closed (the `while` above), so one leftover permit suffices however often it opens and closes meanwhile and ends the next
        //   wait at once if the gate opened before it. The wake is outside the lock (the woken side takes it right away).
        if opened {
            self.reader_room.notify_one();
            self.holder_room.notify_one();
        }
    }
}

/// Share of bytes held by one item. When it drops it subtracts from its own tally (and from the window share too, if it is a window share).
pub(crate) struct Charge {
    ledger: Option<Arc<Ledger>>,
    bytes: usize,
    /// Whether it stands in the window share.
    held: bool,
}

impl Charge {
    /// Share of an item that holds no bytes (a lifecycle event · a report).
    pub(crate) fn none() -> Self {
        Self {
            ledger: None,
            bytes: 0,
            held: false,
        }
    }

    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    /// Counts this share as a window share — when the supervisor holds a fragment in the resume window. The fill level is unchanged and the read gate may open (waking); no change if already a window share or empty.
    pub(crate) fn hold(&mut self) {
        if let (Some(ledger), false) = (&self.ledger, self.held) {
            let bytes = self.bytes;
            ledger.update(|c| c.held += bytes);
            self.held = true;
        }
    }

    /// Subtracts from the window share — called when releasing from the window. The fill level stays the same and the read gate may close (that time is recorded).
    pub(crate) fn unhold(&mut self) {
        if let (Some(ledger), true) = (&self.ledger, self.held) {
            let bytes = self.bytes;
            ledger.update(|c| c.held -= bytes);
            self.held = false;
        }
    }
}

impl Drop for Charge {
    fn drop(&mut self) {
        if let Some(ledger) = self.ledger.take() {
            let (bytes, held) = (self.bytes, self.held);
            ledger.update(|c| {
                c.level -= bytes;
                if held {
                    c.held -= bytes;
                }
            });
        }
    }
}

pub(crate) struct Charged<T> {
    pub item: T,
    pub charge: Charge,
}

impl<T> Charged<T> {
    pub(crate) fn bare(item: T) -> Self {
        Self {
            item,
            charge: Charge::none(),
        }
    }

    pub(crate) fn into_item(self) -> T {
        let Self { item, charge } = self;
        drop(charge);
        item
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::ManualClock;
    use futures_util::future::poll_immediate;
    use std::time::Duration;

    /// Stream-using budget 20 = read share 10 · window cap 10.
    fn ledger() -> (Arc<Ledger>, ManualClock) {
        let clock = ManualClock::new();
        let ledger = Ledger::new(20, clock.handle());
        ledger.use_streams();
        (ledger, clock)
    }

    async fn woken(waiter: tokio::task::JoinHandle<()>, what: &str) {
        tokio::time::timeout(Duration::from_secs(5), waiter)
            .await
            .unwrap_or_else(|_| panic!("the gate opened but did not wake the {what} side"))
            .expect("the waiting task panicked");
    }

    #[test]
    fn the_budget_splits_in_half_and_the_read_share_never_hits_zero() {
        assert_eq!(split(16 << 20), (8 << 20, 8 << 20));
        assert_eq!(split(100), (50, 50));
        assert_eq!(split(3), (1, 2));
        assert_eq!(
            split(1),
            (1, 0),
            "with a read share of 0 the gate never opens"
        );
    }

    #[test]
    fn a_charge_gives_its_bytes_back_when_dropped() {
        let (ledger, _clock) = ledger();
        let a = ledger.charge(4);
        let b = ledger.charge(7);
        assert_eq!(ledger.level(), 11);
        assert!(ledger.over());
        drop(a);
        assert_eq!(ledger.level(), 7);
        assert!(!ledger.over());
        drop(b);
        assert_eq!(ledger.level(), 0);
    }

    #[test]
    fn a_zero_byte_item_holds_no_share_of_the_ledger() {
        let (ledger, _clock) = ledger();
        let none = ledger.charge(0);
        assert_eq!(
            Arc::strong_count(&ledger),
            1,
            "an empty share holds no count"
        );
        drop(none);
        assert_eq!(ledger.level(), 0);
    }

    #[test]
    fn taking_the_item_out_releases_its_share() {
        let (ledger, _clock) = ledger();
        let carried = Charged {
            item: "x",
            charge: ledger.charge(3),
        };
        assert_eq!(ledger.level(), 3);
        assert_eq!(carried.into_item(), "x");
        assert_eq!(ledger.level(), 0);
    }

    // ★The gate opens while both sides are asleep★ — opened before they sleep, the gate check releases them first and the wake-up goes unmeasured.
    #[tokio::test]
    async fn opening_the_gate_wakes_both_the_reader_and_the_holder() {
        let (ledger, _clock) = ledger();
        let first = ledger.charge(6);
        let second = ledger.charge(6);
        let reader = {
            let ledger = ledger.clone();
            tokio::spawn(async move { ledger.reader_room().await })
        };
        let holder = {
            let ledger = ledger.clone();
            tokio::spawn(async move { ledger.holder_room().await })
        };
        crate::testing::settle().await;
        assert!(
            !reader.is_finished() && !holder.is_finished(),
            "waits if at or above the read share"
        );

        drop(first);
        woken(reader, "read").await;
        woken(holder, "supervisor").await;
        drop(second);
    }

    #[test]
    fn the_window_cap_check_counts_the_bytes_about_to_be_added() {
        let (ledger, _clock) = ledger();
        let mut held = ledger.charge(6);
        held.hold();
        assert!(
            ledger.window_fits(4),
            "filling up completely is not exceeding"
        );
        assert!(!ledger.window_fits(5));
        let _gated = ledger.charge(9);
        assert!(
            ledger.window_fits(4),
            "★the window cap measures only the window share★ — a full read share does not reduce the room to hold"
        );
    }

    // ★A fragment counted as a window share does not stop the read gate★ — held fragments must not block reading the replay that would fill their hole; the fill level is unchanged.
    #[tokio::test]
    async fn holding_a_charge_takes_it_out_of_the_gate_but_not_out_of_the_level() {
        let (ledger, _clock) = ledger();
        let mut chunk = ledger.charge(12);
        let reader = {
            let ledger = ledger.clone();
            tokio::spawn(async move { ledger.reader_room().await })
        };
        crate::testing::settle().await;
        assert!(!reader.is_finished(), "waits if at or above the read share");

        chunk.hold();
        assert_eq!((ledger.level(), ledger.held()), (12, 12));
        assert!(!ledger.over());
        woken(reader, "read").await;

        chunk.hold();
        assert_eq!(
            (ledger.level(), ledger.held()),
            (12, 12),
            "unchanged if already in the window share"
        );
        chunk.unhold();
        assert_eq!((ledger.level(), ledger.held()), (12, 0));
        assert!(
            ledger.over(),
            "the share coming out of the window sets up the gate again"
        );
        chunk.hold();
        drop(chunk);
        assert_eq!(
            (ledger.level(), ledger.held()),
            (0, 0),
            "dropped while in the window share, it leaves both"
        );
    }

    #[test]
    fn the_gate_remembers_when_it_shut_until_it_opens() {
        let (ledger, clock) = ledger();
        let t0 = clock.now();
        let small = ledger.charge(5);
        assert_eq!(ledger.take_shut().since, None);

        clock.advance(Duration::from_secs(3));
        let big = ledger.charge(6);
        assert_eq!(
            ledger.take_shut().since,
            Some(t0 + Duration::from_secs(3)),
            "★it is the very moment the line was crossed★ — even if the supervisor notices later, it counts as held from this moment"
        );
        clock.advance(Duration::from_secs(1));
        let _more = ledger.charge(1);
        assert_eq!(
            ledger.take_shut().since,
            Some(t0 + Duration::from_secs(3)),
            "adding while closed keeps the first closing time"
        );

        drop(big);
        assert_eq!(ledger.take_shut().since, None, "cleared once opened");

        let mut chunk = ledger.charge(4);
        chunk.hold();
        clock.advance(Duration::from_secs(2));
        chunk.unhold();
        assert_eq!(
            ledger.take_shut().since,
            Some(t0 + Duration::from_secs(6)),
            "even when taken out of the window and closed, that time is recorded"
        );
        drop(small);
    }

    // ★Gather closed-and-reopened stretches until the supervisor asks★ — if the gate opens and closes while it waits to write, the loop start sees only an open gate; lost time would count toward the window
    //   deadline and fold a fillable hole into truncation.
    #[test]
    fn the_gate_adds_up_every_interval_it_was_shut_until_asked() {
        let (ledger, clock) = ledger();
        let at = |secs| clock.now() + Duration::from_secs(secs);
        let (t1, t10, t11, t13) = (at(1), at(10), at(11), at(13));

        clock.advance(Duration::from_secs(1));
        let first = ledger.charge(10);
        clock.advance(Duration::from_secs(2));
        drop(first);
        clock.advance(Duration::from_secs(4));
        let second = ledger.charge(10);
        clock.advance(Duration::from_secs(3));
        drop(second);
        clock.advance(Duration::from_secs(1));
        let third = ledger.charge(10);
        assert_eq!(
            ledger.take_shut(),
            Shut {
                since: Some(t11),
                closed: Closed {
                    total: Duration::from_secs(5),
                    from: Some(t1),
                    until: Some(t10),
                },
            },
            "★counts both sections (2 s · 3 s)★ — the section closed now is announced only by its start time"
        );
        assert_eq!(
            ledger.take_shut().closed,
            Closed::default(),
            "a section once handed over is forgotten"
        );

        clock.advance(Duration::from_secs(2));
        drop(third);
        assert_eq!(
            ledger.take_shut(),
            Shut {
                since: None,
                closed: Closed {
                    total: Duration::from_secs(2),
                    from: Some(t11),
                    until: Some(t13),
                },
            },
            "a section open at query time is handed over whole after it opens — the side that saw its start earlier through `since` picks it apart so it does not overlap"
        );
    }

    // ★Before a stream fragment the whole budget is the read share and the window cap is 0★ (ADR-0310 decision 16): a non-stream peer's gate is not halved. After the first fragment it halves, and if more than half
    //   is already held the gate closes at once.
    #[test]
    fn the_whole_budget_is_the_read_share_until_streams_are_used() {
        let clock = ManualClock::new();
        let ledger = Ledger::new(20, clock.handle());
        let below = ledger.charge(19);
        assert!(!ledger.over(), "below the budget the gate is open");
        assert!(!ledger.window_fits(1), "window cap 0 — no room to hold");
        let full = ledger.charge(1);
        assert!(ledger.over(), "closes when it reaches the budget");
        drop(full);

        clock.advance(Duration::from_secs(1));
        ledger.use_streams();
        assert!(
            ledger.over(),
            "the read share is halved and it closes on the spot"
        );
        assert_eq!(ledger.take_shut().since, Some(clock.now()));
        assert!(ledger.window_fits(10));
        assert!(!ledger.window_fits(11));
        drop(below);
        assert!(!ledger.over());
    }

    #[tokio::test]
    async fn below_the_read_share_there_is_nothing_to_wait_for() {
        let (ledger, _clock) = ledger();
        let _held = ledger.charge(9);
        assert!(poll_immediate(ledger.reader_room()).await.is_some());
        assert!(poll_immediate(ledger.holder_room()).await.is_some());
    }
}
