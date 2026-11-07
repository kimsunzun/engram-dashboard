// ADR-0269

#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(any(test, feature = "test-support"))]
use std::time::Duration;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Wall-clock epoch milliseconds. If the clock is before 1970 it is 0 — it does not return a failure.
pub fn now_epoch_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The 「지금 읽기」 (read now) seam — lets a test move the time by hand to measure decisions that split by time, without real-time waiting
/// (ADR-0269 decision 3-1).
///
/// ★Waiting is not put here★ (ADR-0269 decision 3-2) — a side that needs sleep makes this the supertrait of its own trait and
/// adds `sleep`. A blocking sleep · a future sleep · a thread launch differ in shape for each user, so they are not
/// bound into one trait.
// ADR-0275
pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

/// The production clock — [`Instant::now`].
///
/// ★No lock and no waiting★ — so it plugs in as is even at sites that read the clock while holding another lock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// A hand-moved test clock (behind the `test-support` feature · ADR-0275 decision 5). It stays stopped at the time of creation and
/// moves forward only through [`ManualClock::advance`].
///
/// ★No lock★ — so it plugs in as is even at sites that read the clock while holding another lock.
/// ★No `Clone`★ — a copy becomes a different clock that flows separately. To share one, wrap it in `Arc`.
#[cfg(any(test, feature = "test-support"))]
pub struct ManualClock {
    origin: Instant,
    offset_nanos: AtomicU64,
}

#[cfg(any(test, feature = "test-support"))]
impl ManualClock {
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
            offset_nanos: AtomicU64::new(0),
        }
    }

    /// Moves forward by `by`. If the accumulated amount exceeds `u64` nanoseconds (about 584 years) it panics — a loud failure is better than wrapping around and the time
    /// going backwards.
    pub fn advance(&self, by: Duration) {
        let by = u64::try_from(by.as_nanos())
            .expect("ManualClock: 한 번에 미는 폭이 u64 나노초를 넘는다");
        self.offset_nanos
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |cur| {
                cur.checked_add(by)
            })
            .expect("ManualClock: 쌓인 폭이 u64 나노초를 넘는다");
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Default for ManualClock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Clock for ManualClock {
    fn now(&self) -> Instant {
        self.origin + Duration::from_nanos(self.offset_nanos.load(Ordering::SeqCst))
    }
}

#[cfg(test)]
mod tests {
    use super::{now_epoch_ms, Clock, ManualClock, SystemClock};
    use std::sync::Arc;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    fn wall_ms() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("시험 기계의 시계는 1970 뒤다")
            .as_millis() as i64
    }

    /// Measures that the unit is milliseconds — seconds or microseconds could not fall between the before and after reads.
    #[test]
    fn the_reading_falls_between_two_wall_clock_reads_in_milliseconds() {
        let before = wall_ms();
        let now = now_epoch_ms();
        let after = wall_ms();
        assert!(
            before <= now && now <= after,
            "{before} <= {now} <= {after}"
        );
    }

    #[test]
    fn the_reading_is_after_2020() {
        assert!(now_epoch_ms() > 1_577_836_800_000);
    }

    #[test]
    fn the_system_clock_reads_the_monotonic_now() {
        let before = Instant::now();
        let now = SystemClock.now();
        let after = Instant::now();
        assert!(before <= now && now <= after);
    }

    #[test]
    fn a_manual_clock_stands_still_until_advanced() {
        let clock = ManualClock::new();
        let first = clock.now();
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(clock.now(), first);
    }

    #[test]
    fn advancing_adds_exactly_the_given_span_and_accumulates() {
        let clock = ManualClock::new();
        let start = clock.now();
        clock.advance(Duration::from_secs(11));
        assert_eq!(clock.now() - start, Duration::from_secs(11));
        clock.advance(Duration::from_nanos(1));
        assert_eq!(
            clock.now() - start,
            Duration::from_secs(11) + Duration::from_nanos(1)
        );
    }

    /// Shared handles see the same time — production receives it as `Arc<dyn Clock>` and the test moves it through the original.
    #[test]
    fn a_shared_handle_sees_an_advance_made_on_another_thread() {
        let clock = Arc::new(ManualClock::new());
        let seen: Arc<dyn Clock> = clock.clone();
        let start = seen.now();
        let pusher = Arc::clone(&clock);
        std::thread::spawn(move || pusher.advance(Duration::from_secs(3)))
            .join()
            .expect("미는 스레드");
        assert_eq!(seen.now() - start, Duration::from_secs(3));
    }

    #[test]
    #[should_panic(expected = "쌓인 폭")]
    fn an_advance_beyond_the_counter_panics_instead_of_wrapping() {
        let clock = ManualClock::new();
        clock.advance(Duration::from_nanos(u64::MAX));
        clock.advance(Duration::from_nanos(1));
    }
}
