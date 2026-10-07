//! 락 오염(poison) 되찾기 — 오염된 자물쇠에서도 가드를 그대로 돌려준다. 경고는 내지 않는다.
//!
//! 계약 셋:
//! 1. **오염 표시를 걷지 않는다**(`clear_poison` 을 부르지 않는다) — 같은 자물쇠를 `.expect` · `try_lock`
//!    으로 쓰는 이웃이 보는 신호를 지우지 않는다. 표시를 걷어야 하는 자리는 이 모듈을 쓰지 않고 자기
//!    도우미를 둔다(ADR-0231 · ADR-0275).
//! 2. **로그 · 다른 락 · 밖 호출을 하지 않는다** — 더하는 동작이 없으므로, 「쥔 채 로그 · IO · 밖 호출
//!    금지」 규율이 걸린 자물쇠의 자리가 이것을 불러도 그 규율이 그대로다. 무엇을 언제 잡는지는 부르는
//!    쪽 몫이다.
//! 3. **운영 빌드는 이 갈래에 닿지 않는다** — 워크스페이스 `[profile.release]` 가 `panic = "abort"` 라
//!    오염이 생기지 않는다. debug · 시험 빌드의 복구다.
//!
//! ★경고를 더하지 말 것★ — 경고는 가드를 쥔 채 찍히므로 2가 깨진다. 더하려면 부르는 자리 전체의
//! 「쥔 채 로그 금지」 감사를 처음부터 다시 해야 한다(ADR-0275).
// ADR-0269
// ADR-0275

use std::sync::{
    Condvar, Mutex, MutexGuard, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard,
    WaitTimeoutResult,
};
use std::time::Duration;

pub fn lock<T: ?Sized>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

pub fn read<T: ?Sized>(l: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    l.read().unwrap_or_else(PoisonError::into_inner)
}

pub fn write<T: ?Sized>(l: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    l.write().unwrap_or_else(PoisonError::into_inner)
}

pub fn wait_timeout<'a, T>(
    cv: &Condvar,
    g: MutexGuard<'a, T>,
    d: Duration,
) -> (MutexGuard<'a, T>, WaitTimeoutResult) {
    cv.wait_timeout(g, d)
        .unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::{lock, read, wait_timeout, write};
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::sync::{Condvar, Mutex, RwLock};
    use std::time::Duration;

    fn poison_mutex(m: &Mutex<u32>, left: u32) {
        let r = catch_unwind(AssertUnwindSafe(|| {
            let mut g = m.lock().expect("시험 전제: 아직 깨끗하다");
            *g = left;
            panic!("시험 — 쥔 채 패닉");
        }));
        assert!(r.is_err());
        assert!(m.is_poisoned(), "시험 전제: 자물쇠가 오염됐다");
    }

    // 쓰기 가드로 패닉한다 — 읽기 가드를 쥔 채의 패닉은 `RwLock` 을 오염시키지 않는다.
    fn poison_rwlock(l: &RwLock<u32>, left: u32) {
        let r = catch_unwind(AssertUnwindSafe(|| {
            let mut g = l.write().expect("시험 전제: 아직 깨끗하다");
            *g = left;
            panic!("시험 — 쥔 채 패닉");
        }));
        assert!(r.is_err());
        assert!(l.is_poisoned(), "시험 전제: 자물쇠가 오염됐다");
    }

    // ── 깨끗한 자물쇠 ──

    #[test]
    fn a_clean_lock_gives_its_guard_and_stays_clean() {
        let m = Mutex::new(1);
        *lock(&m) += 1;
        assert_eq!(*lock(&m), 2);
        assert!(!m.is_poisoned());

        let l = RwLock::new(1);
        *write(&l) += 1;
        assert_eq!(*read(&l), 2);
        assert!(!l.is_poisoned());
    }

    #[test]
    fn a_clean_wait_returns_the_guard() {
        let m = Mutex::new(5);
        let cv = Condvar::new();
        let (g, _res) = wait_timeout(&cv, lock(&m), Duration::from_millis(1));
        assert_eq!(*g, 5);
        drop(g);
        assert!(!m.is_poisoned());
    }

    // ── 오염된 자물쇠 — 되찾고, 값은 패닉 직전 그대로, 표시는 남는다 ──

    #[test]
    fn lock_recovers_a_poisoned_mutex_and_keeps_the_mark() {
        let m = Mutex::new(0);
        poison_mutex(&m, 7);

        let mut g = lock(&m);
        assert_eq!(*g, 7, "패닉 직전에 적은 값이 그대로다");
        *g = 8;
        drop(g);

        assert!(m.is_poisoned(), "되찾아도 오염 표시는 남는다");
        assert_eq!(*lock(&m), 8, "되찾은 가드로 쓴 값이 남는다");
        assert!(m.is_poisoned());
    }

    #[test]
    fn read_and_write_recover_a_poisoned_rwlock_and_keep_the_mark() {
        let l = RwLock::new(0);
        poison_rwlock(&l, 7);

        assert_eq!(*read(&l), 7, "패닉 직전에 적은 값이 그대로다");
        assert!(l.is_poisoned(), "읽기로 되찾아도 오염 표시는 남는다");

        *write(&l) = 8;
        assert!(l.is_poisoned(), "쓰기로 되찾아도 오염 표시는 남는다");
        assert_eq!(*read(&l), 8);
    }

    #[test]
    fn wait_timeout_recovers_a_poisoned_mutex_and_keeps_the_mark() {
        let m = Mutex::new(0);
        let cv = Condvar::new();
        poison_mutex(&m, 7);

        let (g, _res) = wait_timeout(&cv, lock(&m), Duration::from_millis(1));
        assert_eq!(*g, 7);
        drop(g);
        assert!(m.is_poisoned(), "깨어나 되찾아도 오염 표시는 남는다");
    }
}
