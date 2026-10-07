// ADR-0269
// ADR-0275

use std::time::{Duration, Instant};

/// `cond` 가 참이 될 때까지 20 ms 마다 본다. 시한이 지나면 마지막으로 한 번 더 보고 그 값을 돌려준다.
pub fn wait_until(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    cond()
}

#[cfg(test)]
mod tests {
    use super::wait_until;
    use std::time::{Duration, Instant};

    #[test]
    fn an_already_true_condition_is_seen_on_the_first_look() {
        let mut looks = 0;
        assert!(wait_until(Duration::from_secs(5), || {
            looks += 1;
            true
        }));
        assert_eq!(looks, 1);
    }

    /// 시한 0 이면 반복 안에서는 한 번도 안 본다 — 그래도 참을 돌려받는 것이 마감 뒤 한 번의 증거다.
    #[test]
    fn a_zero_timeout_still_takes_the_last_look() {
        let mut looks = 0;
        assert!(wait_until(Duration::ZERO, || {
            looks += 1;
            true
        }));
        assert_eq!(looks, 1);
        assert!(!wait_until(Duration::ZERO, || false));
    }

    #[test]
    fn a_condition_that_turns_true_later_is_caught() {
        let mut looks = 0;
        assert!(wait_until(Duration::from_secs(5), || {
            looks += 1;
            looks >= 3
        }));
        assert_eq!(looks, 3);
    }

    #[test]
    fn a_condition_that_never_turns_true_is_false_after_the_timeout() {
        let timeout = Duration::from_millis(60);
        let started = Instant::now();
        assert!(!wait_until(timeout, || false));
        assert!(started.elapsed() >= timeout);
    }
}
