//! Time seam — lets backoff · keepalive · write deadlines be verified **without actually waiting** (ADR-0177 decision 8).
//!
//! ★Why dyn★ — generic would spread over `Peer` · `Registry` · `Pending` · `Streams` signatures and the registry's collection type; one production implementation gains nothing from monomorphization.
//! ★Why not `tokio::time::pause()`★ — ① it is **runtime-global** and cannot apply to [`crate::machine`] (a pure layer with no future) ② transport and time should stay separately swappable ③ `test-util` is dev-only and
//! may be off in the consumer harness. In real-socket tests `pause()` is cheaper, so it is not forbidden. ★Not yet merged with base `time::Clock`★ — at stage 3 this trait becomes its subtrait; until then this crate does
//! not depend on base (ADR-0275 decision 3).

// ADR-0275
use std::future::Future;
use std::time::{Duration, Instant};

use futures_util::future::{BoxFuture, Either};

/// What time it is and how long to wait. Base's `time::Clock` only reads `now()`; **this crate adds `sleep`** (backoff · keepalive · write deadlines need *waiting*).
pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> Instant;
    /// The returned future is `'static` — it must stay alive after being put in a `select!` arm.
    fn sleep(&self, d: Duration) -> BoxFuture<'static, ()>;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep(&self, d: Duration) -> BoxFuture<'static, ()> {
        Box::pin(tokio::time::sleep(d))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Elapsed;

impl std::fmt::Display for Elapsed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("deadline elapsed")
    }
}

impl std::error::Error for Elapsed {}

/// A deadline running on `dyn Clock`. ★A free function on purpose★ — a generic trait method would make the trait not dyn-compatible and collapse the time axis. If `d` is 0, `f` is **polled once**, so an
/// already-passed deadline does not unconditionally cut off work in progress.
pub async fn with_deadline<F: Future>(
    clock: &dyn Clock,
    d: Duration,
    f: F,
) -> Result<F::Output, Elapsed> {
    let sleep = clock.sleep(d);
    futures_util::pin_mut!(f);
    match futures_util::future::select(f, sleep).await {
        Either::Left((out, _)) => Ok(out),
        Either::Right((_, _)) => Err(Elapsed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::ManualClock;

    #[tokio::test]
    async fn deadline_returns_the_value_when_the_work_finishes_first() {
        let clock = ManualClock::new();
        let out = with_deadline(&clock, Duration::from_secs(5), async { 7u8 }).await;
        assert_eq!(out, Ok(7));
    }

    #[tokio::test]
    async fn deadline_fires_without_any_real_waiting() {
        let clock = ManualClock::new();
        let started = clock.now();
        let never = clock.sleep(Duration::from_secs(3600));
        let handle = clock.clone();
        let work = async move {
            never.await;
            0u8
        };
        let fut = with_deadline(&handle, Duration::from_secs(5), work);
        futures_util::pin_mut!(fut);
        assert!(futures_util::poll!(fut.as_mut()).is_pending());
        clock.advance(Duration::from_secs(5));
        assert_eq!(fut.await, Err(Elapsed));
        assert_eq!(clock.now().duration_since(started), Duration::from_secs(5));
    }

    #[tokio::test]
    async fn manual_clock_does_not_move_on_its_own() {
        let clock = ManualClock::new();
        let t0 = clock.now();
        tokio::task::yield_now().await;
        assert_eq!(clock.now(), t0);
    }
}
