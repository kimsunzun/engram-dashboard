//! Lock poison recovery — returns the guard as is even from a poisoned lock. It emits no warning.
//!
//! Three contracts:
//! 1. **It does not clear the poison mark** (it does not call `clear_poison`) — it does not erase the signal that neighbors using the same lock
//!    with `.expect` · `try_lock` see. A site that must clear the mark does not use this module and keeps its own
//!    helper (ADR-0231 · ADR-0275).
//! 2. **No logging · other locks · outside calls** — it adds no behavior, so even when a site of a lock bound by the 「쥔 채 로그 · IO · 밖 호출
//!    금지」 (no log · IO · outside call while holding) discipline calls this, that discipline stays as it is. What is taken and when is up to the
//!    caller.
//! 3. **In production processes (daemon · shell), the only poison that reaches this branch is what a panic inside a contained unit left** — a panic outside the 「가두는 중」
//!    (containing) mark ends the process in the hook before unwinding, so no poison arises. If a contained unit (the daemon's three agent pumps ·
//!    input writer — bus command execution and the shell are not contained · ADR-0293) panics while holding a lock, that lock stays poisoned
//!    and the process keeps running (the reason line of that panic is left by the hook regardless of the mark). A caller that then takes it here gets the value from just before
//!    the panic — it may be half-fixed, and whether that is acceptable is the caller's call. In a test build without the hook,
//!    poison from anywhere comes to this branch.
//!
//! ★Do not add a warning★ — a warning is printed while holding the guard, so 2 breaks. To add one, the whole of the calling sites'
//! 「쥔 채 로그 금지」 (no logging while holding) audit would have to be redone from scratch (ADR-0275).
// ADR-0269
// ADR-0275
// ADR-0292

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

    // Panics with a write guard — a panic while holding a read guard does not poison an `RwLock`.
    fn poison_rwlock(l: &RwLock<u32>, left: u32) {
        let r = catch_unwind(AssertUnwindSafe(|| {
            let mut g = l.write().expect("시험 전제: 아직 깨끗하다");
            *g = left;
            panic!("시험 — 쥔 채 패닉");
        }));
        assert!(r.is_err());
        assert!(l.is_poisoned(), "시험 전제: 자물쇠가 오염됐다");
    }

    // ── Clean lock ──

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

    // ── Poisoned lock — recovered, the value is exactly as just before the panic, the mark remains ──

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
