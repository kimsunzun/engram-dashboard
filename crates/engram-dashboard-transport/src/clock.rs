//! 시간 seam — 백오프·keepalive·쓰기 시한이 **실제로 기다리지 않고** 검증되게 하는 축(ADR-0177 결정 8).
//!
//! ★왜 dyn 인가★ — 제네릭으로 두면 `Peer`·`Registry`·`Pending`·`Streams` 네 타입 시그니처에 전부 번지고
//! 명부의 컬렉션 타입까지 오염된다. 운영 구현이 하나뿐이라 단형화로 얻을 것도 없다.
//!
//! ★왜 `tokio::time::pause()` 로 대신하지 않나★ — ① 그것은 **런타임 전역** 장치라 런타임을 세워야만 쓰고
//! [`crate::machine`](future 가 하나도 없는 순수 층)에는 적용할 데가 없다 ② 전송도 갈아끼우기로 한
//! 마당에 시간을 tokio 에 묶으면 그 둘이 도로 붙는다 ③ `test-util` 은 dev 전용이라 소비자 하네스에서
//! 켜지지 않을 수 있다. **단 실소켓 테스트에서는 `pause()` 가 더 싸므로 금지하지 않는다.**

use std::future::Future;
use std::time::{Duration, Instant};

use futures_util::future::{BoxFuture, Either};

/// 지금 몇 시인가와 얼마나 기다리나.
///
/// 선례는 `daemon` 의 `command_delivery::Clock`(`now()` 하나)이고 **이 crate 는 거기에 `sleep` 을
/// 더한다** — 백오프·keepalive·쓰기 시한은 *기다림*이 필요한데 `now()` 만으로는 그것을 못 만든다.
pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> Instant;
    /// 반환 future 가 `'static` 인 것이 계약이다 — `select!` 팔에 넣은 뒤 그 자리에서 살아 있어야 한다.
    fn sleep(&self, d: Duration) -> BoxFuture<'static, ()>;
}

/// 운영 구현.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep(&self, d: Duration) -> BoxFuture<'static, ()> {
        Box::pin(tokio::time::sleep(d))
    }
}

/// 시한이 지났다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Elapsed;

impl std::fmt::Display for Elapsed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("deadline elapsed")
    }
}

impl std::error::Error for Elapsed {}

/// `dyn Clock` 위에서 도는 시한.
///
/// ★trait 메서드가 아니라 자유 함수인 것은 의도★ — 제네릭 메서드를 trait 에 두면 그 trait 이 dyn 호환이
/// 아니게 되어 시간 축 전체가 무너진다.
///
/// `d` 가 0이면 `f` 를 **한 번은 폴링한다** — 이미 지난 시한이 진행 중인 일을 무조건 잘라내지 않게 한다.
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
            // 시한이 걸린 쪽은 영원히 안 끝난다.
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
