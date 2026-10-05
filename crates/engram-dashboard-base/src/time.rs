// ADR-0269

#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(any(test, feature = "test-support"))]
use std::time::Duration;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// 벽시계 epoch 밀리초. 시계가 1970 보다 앞이면 0 이다 — 실패를 돌려주지 않는다.
pub fn now_epoch_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 「지금 읽기」 seam — 시각으로 갈리는 판정을 시험이 실시간 대기 없이 시각을 손으로 밀어 재게 한다
/// (ADR-0269 결정 3-1).
///
/// ★기다리기는 여기 두지 않는다★(ADR-0269 결정 3-2) — 잠이 필요한 쪽은 자기 트레이트가 이것을 상위
/// 트레이트로 두고 `sleep` 을 더한다. 막는 잠 · future 잠 · 스레드 기동처럼 쓰는 쪽마다 모양이 달라 한
/// 트레이트로 묶이지 않는다.
// ADR-0275
pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

/// 운영 시계 — [`Instant::now`].
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// 손으로 미는 시험용 시계(`test-support` 기능 뒤 · ADR-0275 결정 5). 만든 순간의 시각에 멈춰 있고
/// [`ManualClock::advance`] 로만 앞으로 간다.
///
/// ★자물쇠가 없다★ — 다른 자물쇠를 쥔 채 시계를 읽는 자리에도 그대로 꽂히게 한다.
/// ★`Clone` 을 달지 않는다★ — 복제본은 따로 흐르는 다른 시계가 된다. 나눠 쥐려면 `Arc` 로 감싼다.
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

    /// `by` 만큼 앞으로 민다. 쌓인 폭이 `u64` 나노초(약 584년)를 넘으면 패닉한다 — 감아 돌아 시각이 뒤로
    /// 가는 것보다 시끄러운 실패가 낫다.
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

    /// 단위가 밀리초인지 잰다 — 초나 마이크로초면 앞뒤 읽기 사이에 들지 못한다.
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

    /// 나눠 쥔 손잡이가 같은 시각을 본다 — 운영 쪽이 `Arc<dyn Clock>` 으로 받고 시험이 원본으로 민다.
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
