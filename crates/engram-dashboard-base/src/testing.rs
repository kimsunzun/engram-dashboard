// ADR-0269
// ADR-0275

use std::time::{Duration, Instant};

/// Checks every 20 ms until `cond` becomes true. After the deadline it checks once more at the end and returns that value.
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

    /// With a deadline of 0 it never checks inside the loop — still, getting true back is the evidence of the single check after the deadline.
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
