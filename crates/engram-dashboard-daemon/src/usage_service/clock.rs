//! 사용량 판정의 시계 seam — 쿨타임·거절 대기·값 나이는 [`UsageClock::mono`], 리셋 비교·거절
//! 파일은 [`UsageClock::wall`] 로 잰다(TRD §3 #28). 서비스의 판정은 늘 이 시계로 잰다 — 대기 타이머가 깨운
//! 시각은 믿지 않는다.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// ★구현은 서비스·명부·책을 다시 부르지 않는다★ — 서비스는 책 락을 쥔 채(구독 교체의 첫 한 장에서는 명부 →
/// 책 락을 쥔 채) 이 시계를 읽는다. 다시 부르면 교착이다.
pub trait UsageClock: Send + Sync + 'static {
    /// 이 시계의 기점부터 흐른 단조 시간 — 벽시계가 되감겨도 줄지 않는다. 잠든 시간은 OS 가 세는 만큼 센다.
    fn mono(&self) -> Duration;
    /// 벽시계, epoch 초. 되감길 수 있다. 1970 전이면 음수이고 패닉하지 않는다.
    fn wall(&self) -> i64;
}

/// 실물 시계. `mono` 의 기점 = 만든 순간.
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
        // `Instant` 가 잠든 시간을 세는지는 OS 마다 다르다고 알려져 있다 — Windows QPC 는 세고 Linux
        // `CLOCK_MONOTONIC`·macOS 는 안 센다(미검 · TRD §6 #2). 안 세면 복귀 뒤 쿨타임·값 나이가 잠든 만큼
        // 늦게 흐른다. OS 로 가를 때는 이 함수 안에서만 가른다.
        // ADR-0230
        self.origin.elapsed()
    }

    fn wall(&self) -> i64 {
        epoch_secs(SystemTime::now())
    }
}

/// 초 내림(floor) — 1970 전 0.5초 = -1 이지 0 이 아니다. i64 를 넘는 쪽은 양·음 끝에서 포화한다.
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

/// 시험용 손 시계. `mono` 와 `wall` 을 따로 옮길 수 있다 — 벽시계만 되감기·단조 시간만 흐르기를 재현한다.
#[cfg(test)]
pub(crate) struct ManualUsageClock {
    state: std::sync::Mutex<ManualState>,
}

#[cfg(test)]
struct ManualState {
    mono: Duration,
    wall: i64,
    /// `advance_both` 가 아직 `wall` 에 못 옮긴 1초 미만 몫 — 0.5초씩 두 번이면 `wall` 이 1 오른다.
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

    /// `mono` 만 옮긴다 — `wall` 은 그대로.
    pub(crate) fn advance(&self, by: Duration) {
        let mut s = self.lock();
        s.mono = s.mono.saturating_add(by);
    }

    /// 둘을 함께 옮긴다 — `wall` 은 1초 미만 몫을 모아 정수 초로 옮긴다.
    pub(crate) fn advance_both(&self, by: Duration) {
        let mut s = self.lock();
        s.mono = s.mono.saturating_add(by);
        let carried = s.wall_carry.saturating_add(by);
        let whole = i64::try_from(carried.as_secs()).unwrap_or(i64::MAX);
        s.wall = s.wall.saturating_add(whole);
        s.wall_carry = Duration::from_nanos(u64::from(carried.subsec_nanos()));
    }

    /// `wall` 만 그 값으로 둔다(되감기 포함) — 모아 둔 1초 미만 몫도 버린다.
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

        // 벽시계만 되감긴다 — 단조 시간은 그대로다.
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
        // set_wall 은 모아 둔 0.7초를 버린다 — 0.5초 더해도 1초가 안 된다.
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
            panic!("락을 쥔 채 죽는다");
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
            .expect("시험 기계의 벽시계는 1970 뒤다")
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
            .expect("이 OS 의 SystemTime 은 1970 전 5초를 나타낸다");
        assert_eq!(epoch_secs(before), -5);
        let half = UNIX_EPOCH
            .checked_sub(Duration::from_millis(500))
            .expect("이 OS 의 SystemTime 은 1970 전 0.5초를 나타낸다");
        assert_eq!(epoch_secs(half), -1);
        let past_five = before
            .checked_sub(Duration::from_millis(1))
            .expect("1970 전 5.001초");
        assert_eq!(epoch_secs(past_five), -6);
    }
}
