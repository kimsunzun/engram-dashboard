//! The clock seam of usage decisions — cooldown · refusal wait · value age are measured with [`UsageClock::mono`], reset comparison with [`UsageClock::wall`] (TRD §3 #28 · ADR-0250). The service's decisions are always measured by this clock — the time a wait timer woke it is not trusted.
//!
//! ★Not merged with base `time::Clock`★ (ADR-0275 decision 7) — `mono` is a `Duration` from the origin so it flows straight into the book's time type and tests can measure `Duration::MAX` · `ZERO` boundaries; `wall` is signed epoch seconds. A current-time read returning `Instant` cannot hold both.

// ADR-0275
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// ★An implementation does not call back into the service, registry or book★ — the service reads this clock while holding the book lock (in a subscription replacement's first sheet, while holding registry → book). Calling back is a deadlock.
pub trait UsageClock: Send + Sync + 'static {
    /// Monotonic time since this clock's origin — does not shrink if the wall clock rewinds. Slept time counts as much as the OS counts it.
    fn mono(&self) -> Duration;
    /// Wall clock, epoch seconds. Can rewind. Before 1970 it is negative and does not panic.
    fn wall(&self) -> i64;
}

/// The real clock. The origin of `mono` = the moment it was made.
pub struct OsUsageClock {
    origin: Instant,
}

impl OsUsageClock {
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Default for OsUsageClock {
    fn default() -> Self {
        Self::new()
    }
}

impl UsageClock for OsUsageClock {
    fn mono(&self) -> Duration {
        // Whether `Instant` counts slept time differs by OS — Windows QPC counts it, Linux `CLOCK_MONOTONIC` and macOS do not (unverified · TRD §6 #2). If it does not, after resume the cooldown and value age run behind by the slept time. The OS split lives only inside this function.
        // ADR-0230
        self.origin.elapsed()
    }

    fn wall(&self) -> i64 {
        epoch_secs(SystemTime::now())
    }
}

/// Seconds rounded down (floor) — 0.5 s before 1970 = -1, not 0. Saturates at the i64 ends.
fn epoch_secs(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_secs()).unwrap_or(i64::MAX),
        Err(before) => {
            let gap = before.duration();
            let whole = gap
                .as_secs()
                .saturating_add(u64::from(gap.subsec_nanos() > 0));
            i64::try_from(whole).map_or(i64::MIN, |s| -s)
        }
    }
}

/// Hand-driven clock for tests. `mono` and `wall` move separately — reproduces the wall clock rewinding alone and only monotonic time flowing.
#[cfg(test)]
pub(crate) struct ManualUsageClock {
    state: std::sync::Mutex<ManualState>,
}

#[cfg(test)]
struct ManualState {
    mono: Duration,
    wall: i64,
    /// The sub-1-second share `advance_both` could not yet move into `wall` — two times 0.5 s raises `wall` by 1.
    wall_carry: Duration,
}

#[cfg(test)]
impl ManualUsageClock {
    pub(crate) fn new(mono: Duration, wall: i64) -> Self {
        Self {
            state: std::sync::Mutex::new(ManualState {
                mono,
                wall,
                wall_carry: Duration::ZERO,
            }),
        }
    }

    /// Moves only `mono` — `wall` stays.
    pub(crate) fn advance(&self, by: Duration) {
        let mut s = self.lock();
        s.mono = s.mono.saturating_add(by);
    }

    /// Moves both — `wall` gathers the sub-1-second share and moves in whole seconds.
    pub(crate) fn advance_both(&self, by: Duration) {
        let mut s = self.lock();
        s.mono = s.mono.saturating_add(by);
        let carried = s.wall_carry.saturating_add(by);
        let whole = i64::try_from(carried.as_secs()).unwrap_or(i64::MAX);
        s.wall = s.wall.saturating_add(whole);
        s.wall_carry = Duration::from_nanos(u64::from(carried.subsec_nanos()));
    }

    /// Sets only `wall` (including rewinding) — the gathered sub-1-second share is discarded too.
    pub(crate) fn set_wall(&self, wall: i64) {
        let mut s = self.lock();
        s.wall = wall;
        s.wall_carry = Duration::ZERO;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ManualState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
impl UsageClock for ManualUsageClock {
    fn mono(&self) -> Duration {
        self.lock().mono
    }

    fn wall(&self) -> i64 {
        self.lock().wall
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_moves_mono_and_wall_independently() {
        let clock = ManualUsageClock::new(Duration::from_secs(10), 1_000);
        assert_eq!(
            (clock.mono(), clock.wall()),
            (Duration::from_secs(10), 1_000)
        );

        clock.advance(Duration::from_secs(5));
        assert_eq!(
            (clock.mono(), clock.wall()),
            (Duration::from_secs(15), 1_000)
        );

        clock.advance_both(Duration::from_secs(3));
        assert_eq!(
            (clock.mono(), clock.wall()),
            (Duration::from_secs(18), 1_003)
        );

        // Only the wall clock rewinds — monotonic time stays.
        clock.set_wall(900);
        assert_eq!((clock.mono(), clock.wall()), (Duration::from_secs(18), 900));
    }

    #[test]
    fn manual_clock_advance_both_carries_sub_second_parts_into_wall() {
        let clock = ManualUsageClock::new(Duration::ZERO, 0);
        clock.advance_both(Duration::from_millis(500));
        assert_eq!(clock.wall(), 0);
        clock.advance_both(Duration::from_millis(500));
        assert_eq!((clock.mono(), clock.wall()), (Duration::from_secs(1), 1));
        clock.advance_both(Duration::from_millis(1_700));
        assert_eq!(clock.wall(), 2);
        // set_wall discards the gathered 0.7 s — adding 0.5 s does not make 1 second.
        clock.set_wall(100);
        clock.advance_both(Duration::from_millis(500));
        assert_eq!(clock.wall(), 100);
    }

    #[test]
    fn manual_clock_saturates_instead_of_panicking() {
        let clock = ManualUsageClock::new(Duration::MAX, i64::MAX);
        clock.advance(Duration::from_secs(1));
        clock.advance_both(Duration::MAX);
        assert_eq!((clock.mono(), clock.wall()), (Duration::MAX, i64::MAX));
    }

    #[test]
    fn manual_clock_survives_a_poisoned_lock() {
        let clock = std::sync::Arc::new(ManualUsageClock::new(Duration::ZERO, 5));
        let held = clock.clone();
        let _ = std::thread::spawn(move || {
            let _guard = held.lock();
            panic!("dies while holding the lock");
        })
        .join();
        assert!(clock.state.is_poisoned());
        clock.advance(Duration::from_secs(1));
        assert_eq!((clock.mono(), clock.wall()), (Duration::from_secs(1), 5));
    }

    #[test]
    fn os_clock_mono_does_not_go_backwards() {
        let clock = OsUsageClock::new();
        let first = clock.mono();
        std::thread::sleep(Duration::from_millis(5));
        let second = clock.mono();
        assert!(second >= first, "{first:?} → {second:?}");
    }

    #[test]
    fn os_clock_wall_matches_system_time() {
        let clock = OsUsageClock::default();
        let expected = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the test machine's wall clock is after 1970")
            .as_secs() as i64;
        let got = clock.wall();
        assert!((got - expected).abs() <= 5, "wall {got} vs {expected}");
    }

    #[test]
    fn epoch_secs_is_negative_before_1970() {
        assert_eq!(epoch_secs(UNIX_EPOCH), 0);
        assert_eq!(epoch_secs(UNIX_EPOCH + Duration::from_secs(90)), 90);
        let before = UNIX_EPOCH
            .checked_sub(Duration::from_secs(5))
            .expect("this OS's SystemTime represents 5 seconds before 1970");
        assert_eq!(epoch_secs(before), -5);
        let half = UNIX_EPOCH
            .checked_sub(Duration::from_millis(500))
            .expect("this OS's SystemTime represents 0.5 seconds before 1970");
        assert_eq!(epoch_secs(half), -1);
        let past_five = before
            .checked_sub(Duration::from_millis(1))
            .expect("5.001 seconds before 1970");
        assert_eq!(epoch_secs(past_five), -6);
    }
}
