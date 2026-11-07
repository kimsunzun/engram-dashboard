//! Byte tally of what has been received — ★driving layer★ (TRD 3 §3-0 ① · ADR-0310 decision 17).
//!
//! One tally per supervisor ([`Ledger`]). The moment the read task reads a frame off the link, it adds that payload length
//! ([`Ledger::charge`]); that share ([`Charge`]) travels with the item through the read-ahead queue → ordering area → upward queue (it may
//! be held by the resume window along the way) and is subtracted when it drops — when the receiver takes it out, or when the supervisor discards it (a reply · an overlapping
//! fragment · a fragment the window drops when the answer arrives). ★So one tally covers the three layers and the window★ — measuring each layer separately would let each fill up to its own cap
//! so the sum grows as a product (read-ahead queue 512 entries × the one-message cap is that shape).
//!
//! ★The share rides on the item, not on a layer★ — while an item crosses the read-ahead queue · ordering area · inbox ([`crate::Inbox`]),
//! it must hold the tally of the supervisor that put it in, so that one tally covers all three layers. Dropping the inbox
//! drops the shares along with the items it holds.
//!
//! ★The window share is counted separately inside the tally★ — it is the bytes of stream fragments the resume window holds. The supervisor adds it when holding
//! ([`Charge::hold`]) and subtracts it when releasing from the window or discarding. The whole tally (fill level) stays the same. Budget B is
//! split into a read share and a window cap, and each gets its own cap.
//! - Read gate — if 「물높이 − 창 몫」 (fill level minus window share) is at least the read share, the read task stops reading the link. The decision uses 「더하기 전
//!   물높이」 (the fill level before adding; the same rule as net `CONN_RX_MAX_BYTES`), so while the gate is open one frame gets in whatever its size.
//!   ★Subtracting the window share at the gate is the point★ — if held fragments stopped reading, the replay that would fill their hole could not be read, and
//!   a replay arriving late, after the window closed at its deadline, is discarded as a past number. A hole that could have been filled becomes a permanent loss.
//! - Window cap — the window share never goes over the cap, not even by one frame. A fragment that would exceed it is not held and the window
//!   is closed as truncated (`Supervisor::route`).
//!
//! ★Splitting starts when the first stream fragment is seen ([`Ledger::use_streams`])★ — before that, read share = B · window cap = 0,
//! and after that it is half and half ([`split`]). A peer that does not use the stream layer (ADR-0310 decision 16 — both Wires' `stream_mark` is
//! always `None`) never has anything for the window to hold — splitting in half from the start would leave the window share at 0 and only halve the read gate. `Wire` has
//! no field that announces in advance whether streams are used (adding one would break every consumer — `wire.rs`), so it is learned from the first fragment seen.
//!
//! So the ceiling of the whole tally = read share + window cap + one message = B + M (TRD 3 §3-0 ①). Only a read grows the tally,
//! and a read happens only while the gate is open (fill level < read share + window cap at that moment), one frame at a time — putting into or taking out of the window does not
//! change the fill level, and even if a share taken out of the window after the gate was seen open but before that one frame is read closes the gate again, the fill level stays the same.
//! It also holds at the moment splitting starts — the read share only drops from B to half, so the gate moves only toward closing, and what is held
//! until then is below B + M. The cap for one message is set by the adapter (crate header 「알려진 한계」 (known limits)).
//!
//! ★The tally is changed under one lock★ — three tasks open and close the gate: the read task (adds, then closes) · the supervisor (puts into the window and
//! opens, takes out and closes, discards and opens) · the receiver (takes out and opens). Along with the gate state, the lock records the time the gate closed and the stretches where it
//! closed and reopened ([`Ledger::take_shut`]), so that even if the supervisor notices a hold late, it does not lose the time that hold began, and
//! even if the gate closed and reopened while the supervisor was waiting to write, it does not lose that time.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use tokio::sync::Notify;

use crate::clock::Clock;

/// Splits the budget B of a stream-using supervisor into (read share, window cap). The read share never becomes 0 — at 0 the gate would never
/// open.
///
/// Why half and half = the two must sum to B so that the ceiling of the whole tally is B + M (ADR-0310 decision 17), and half and half is the
/// simplest value that starves neither side (an internal value set by the main session — there is no measured basis).
pub(crate) const fn split(budget: usize) -> (usize, usize) {
    let read = if budget / 2 == 0 { 1 } else { budget / 2 };
    (read, budget.saturating_sub(read))
}

/// Tally of payload bytes held by one supervisor — the window share is counted separately inside it (this file's header).
pub(crate) struct Ledger {
    counts: Mutex<Counts>,
    /// Budget B — the read share and window cap are derived by [`Ledger::caps`] depending on whether streams are used.
    budget: usize,
    /// Clock that records the time the gate closed — it must be the supervisor's own clock to agree with the supervisor's deadline calculation.
    clock: Arc<dyn Clock>,
    /// The read gate opened — the read task's share.
    reader_room: Notify,
    /// The same fact, the supervisor's share. ★The wake-up handle is split in two★ — `notify_one` wakes only one waiter and stores
    /// only one leftover permit, so if the two shared one handle, one side would eat the other's permit and the other would not wake.
    holder_room: Notify,
}

struct Counts {
    /// Whole tally (fill level) — includes the window share.
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

/// Answer of [`Ledger::take_shut`] — how long the read gate was closed since the last query.
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

    /// (read share, window cap) — before streams are used, the whole budget is the read share (this file's header). Either way the sum is at most
    /// the budget, so the ceiling B + M holds (at budget 0 the read share is 1 — so the gate never stays shut forever).
    fn caps(&self, streams: bool) -> (usize, usize) {
        if streams {
            split(self.budget)
        } else {
            (self.budget.max(1), 0)
        }
    }

    // Inside the lock there is only addition · subtraction · reading the time, so the tally stays intact even when poisoned.
    fn counts(&self) -> MutexGuard<'_, Counts> {
        self.counts.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whole tally — includes the window share.
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

    /// If the read gate is closed now, returns the time it closed and the stretches that closed and reopened since the last call, and
    /// forgets those stretches (times by this tally's clock). Called when the supervisor counts how long it held (`Supervisor::track_pause`) — with two
    /// callers, one would not see the stretches the other took.
    pub(crate) fn take_shut(&self) -> Shut {
        let mut c = self.counts();
        Shut {
            since: c.shut_at,
            closed: std::mem::take(&mut c.closed),
        }
    }

    /// A stream fragment was seen — from here on the budget is split in half into read share and window cap (this file's header). Never undone.
    /// The read share only shrinks, so the gate may close but never opens because of this.
    pub(crate) fn use_streams(&self) {
        self.update(|c| c.streams = true);
    }

    /// Whether these bytes can still be held in the window — the window cap decision. Unlike the read gate, it measures 「더한 뒤」 (after adding) — the window share never goes
    /// over the cap, not even by one frame. Before [`Ledger::use_streams`] the cap is 0, so this is always false.
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

    /// Waits until the read gate opens — called by the read task before its next read.
    pub(crate) async fn reader_room(&self) {
        while self.over() {
            self.reader_room.notified().await;
        }
    }

    /// The supervisor's share of the same wait. ★Cancel-safe — hang it on a `select!` arm★: it checks the gate first on every poll.
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
        // ★Wake on every gate opening★ — the woken side looks at the gate again and sleeps again if it is closed (the `while` above). So
        //   however many times the gate opens and closes while it sleeps, one leftover permit is enough, and if the gate opened before the wait, that permit ends the next wait
        //   at once. The wake happens outside the lock — the woken side takes that lock right away.
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

    /// Counts this share as a window share — called when the supervisor holds a fragment in the resume window. The fill level stays the same and the read gate
    /// may open (then it wakes). If it is already a window share or an empty share, nothing changes.
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

/// An item that crosses holding its share.
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

    /// Takes it out — the share drops here (the receiver took it out).
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
            .unwrap_or_else(|_| panic!("문이 열렸는데 {what} 쪽을 안 깨웠다"))
            .expect("기다리던 태스크가 패닉했다");
    }

    #[test]
    fn the_budget_splits_in_half_and_the_read_share_never_hits_zero() {
        assert_eq!(split(16 << 20), (8 << 20, 8 << 20));
        assert_eq!(split(100), (50, 50));
        assert_eq!(split(3), (1, 2));
        assert_eq!(split(1), (1, 0), "읽기 몫이 0 이면 문이 영영 안 열린다");
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
        assert_eq!(Arc::strong_count(&ledger), 1, "빈 몫은 셈을 쥐지 않는다");
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

    // ★The gate opens while both sides are asleep★ — if it were before going to sleep, the gate check would release them first and the wake-up could not be measured.
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
            "읽기 몫 이상이면 기다린다"
        );

        drop(first);
        woken(reader, "읽기").await;
        woken(holder, "감독").await;
        drop(second);
    }

    #[test]
    fn the_window_cap_check_counts_the_bytes_about_to_be_added() {
        let (ledger, _clock) = ledger();
        let mut held = ledger.charge(6);
        held.hold();
        assert!(ledger.window_fits(4), "꽉 차는 것까지는 넘는 것이 아니다");
        assert!(!ledger.window_fits(5));
        let _gated = ledger.charge(9);
        assert!(
            ledger.window_fits(4),
            "★창 상한은 창 몫만 잰다★ — 읽기 몫이 찼다고 붙들 자리가 줄지 않는다"
        );
    }

    // ★A fragment counted as a window share does not stop the read gate★ — this is the concrete case of held fragments not blocking the read of the replay that would fill
    //   their hole. The fill level stays the same (one tally).
    #[tokio::test]
    async fn holding_a_charge_takes_it_out_of_the_gate_but_not_out_of_the_level() {
        let (ledger, _clock) = ledger();
        let mut chunk = ledger.charge(12);
        let reader = {
            let ledger = ledger.clone();
            tokio::spawn(async move { ledger.reader_room().await })
        };
        crate::testing::settle().await;
        assert!(!reader.is_finished(), "읽기 몫 이상이면 기다린다");

        chunk.hold();
        assert_eq!((ledger.level(), ledger.held()), (12, 12));
        assert!(!ledger.over());
        woken(reader, "읽기").await;

        chunk.hold();
        assert_eq!(
            (ledger.level(), ledger.held()),
            (12, 12),
            "이미 창 몫이면 그대로다"
        );
        chunk.unhold();
        assert_eq!((ledger.level(), ledger.held()), (12, 0));
        assert!(ledger.over(), "창에서 나온 몫은 다시 문을 세운다");
        chunk.hold();
        drop(chunk);
        assert_eq!(
            (ledger.level(), ledger.held()),
            (0, 0),
            "창 몫인 채 버려지면 둘 다에서 빠진다"
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
            "★선을 넘은 그 시각이다★ — 감독이 나중에 알아채도 이 시각부터 붙든 것으로 센다"
        );
        clock.advance(Duration::from_secs(1));
        let _more = ledger.charge(1);
        assert_eq!(
            ledger.take_shut().since,
            Some(t0 + Duration::from_secs(3)),
            "닫힌 채 더해도 처음 닫힌 시각 그대로다"
        );

        drop(big);
        assert_eq!(ledger.take_shut().since, None, "열리면 지운다");

        let mut chunk = ledger.charge(4);
        chunk.hold();
        clock.advance(Duration::from_secs(2));
        chunk.unhold();
        assert_eq!(
            ledger.take_shut().since,
            Some(t0 + Duration::from_secs(6)),
            "창에서 꺼내 닫혀도 그 시각을 적는다"
        );
        drop(small);
    }

    // ★Gather the stretches that closed and reopened until the supervisor asks★ — if the gate opens and closes while the supervisor waits to write, the start of the
    //   loop sees only an open gate. If that time were lost, it would be counted toward the window deadline and a hole that could have been filled would fold into truncation.
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
            "★두 구간(2 s · 3 s)을 다 센다★ — 지금 닫힌 구간은 시작 시각으로만 알린다"
        );
        assert_eq!(
            ledger.take_shut().closed,
            Closed::default(),
            "한 번 넘긴 구간은 잊는다"
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
            "물을 때 열려 있던 구간은 열린 뒤 통째로 넘긴다 — 그 시작을 앞서 `since` 로 본 쪽이 겹치지 않게 가려 쓴다"
        );
    }

    // ★Before a stream fragment is seen, the whole budget is the read share and the window cap is 0★ — the read gate of a peer that does not use streams (ADR-0310
    //   decision 16) is not halved. Once a fragment is seen the budget is split in half, and if more than half is already held, the gate closes on the spot.
    #[test]
    fn the_whole_budget_is_the_read_share_until_streams_are_used() {
        let clock = ManualClock::new();
        let ledger = Ledger::new(20, clock.handle());
        let below = ledger.charge(19);
        assert!(!ledger.over(), "예산 밑이면 문이 열려 있다");
        assert!(!ledger.window_fits(1), "창 상한 0 — 붙들 자리가 없다");
        let full = ledger.charge(1);
        assert!(ledger.over(), "예산에 닿으면 닫힌다");
        drop(full);

        clock.advance(Duration::from_secs(1));
        ledger.use_streams();
        assert!(ledger.over(), "읽기 몫이 반으로 줄어 그 자리에서 닫힌다");
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
