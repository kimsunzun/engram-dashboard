//! 하네스 전용 — 기능 플래그 `test-support` 뒤에 산다(ADR-0012).

use std::future::Future;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::task::{Context, Poll, Wake, Waker};

/// 패닉 훅은 **프로세스 전역**이라 잡고 되돌리는 구간이 겹치면 안 된다.
static PANIC_HOOK: Mutex<()> = Mutex::new(());

/// 의도된 패닉의 기본 출력을 끄고 `body` 를 돌린다 — 끝나면 원래 훅을 되돌린다.
///
/// ★한 테스트 바이너리의 테스트들은 스레드로 **동시에** 돈다★ — 훅을 각자 갈아 끼우면 늦게 끝난 쪽이
/// 남의 훅을 원본으로 알고 되돌려 놓거나, 조용해야 할 구간에 남의 패닉 출력이 섞인다. 그 경합을 이
/// 한 자리로 막는다(잠금은 훅 교체 구간 전체를 덮는다).
/// ★구간 안에서는 **무관한 스레드**(같은 바이너리의 다른 테스트)의 패닉 출력도 사라진다★ — 훅이
/// 프로세스 전역이라서다. 그래서 구간은 의도된 패닉이 나는 순간만 덮게 좁게 잡는다. 스레드로 가려
/// 조용히 하지 않는 것은 의도다 — 의도된 패닉이 본문이 띄운 스레드(codex 라이터 · tokio blocking
/// pool)에서 나는 호출자가 있다.
/// ★`body` 밖으로 새는 패닉은 의도된 것이 아니다★ — 그 메시지를 되살려 출력하고 **같은 페이로드**로
/// 다시 던진다. 호출자에게는 평범한 테스트 실패로 보인다.
pub fn with_quiet_panic_hook<T>(body: impl FnOnce() -> T) -> T {
    let _held = PANIC_HOOK.lock().unwrap_or_else(PoisonError::into_inner);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    // ★되돌리기를 `Drop` 으로 옮기지 말 것★ — std 는 패닉 중인 스레드의 `set_hook` 을 거부하고, 그 거부가
    //   unwind 중인 소멸자 안의 두 번째 패닉이 되어 프로세스가 abort 한다(테스트 바이너리가 메시지 없이
    //   통째로 죽는다 — Windows `0xC0000409`, 실측). 그렇다고 대입문만 두면 unwind 가 그 줄을 건너뛰어
    //   조용한 훅이 남는다. 그래서 unwind 를 잡은 **뒤에** 되돌린다.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
    std::panic::set_hook(previous);
    outcome.unwrap_or_else(|payload| {
        let message = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("<문자열이 아닌 패닉 페이로드>");
        eprintln!("조용한 패닉 훅 구간 안에서 본문이 패닉했다(원래 출력은 삼켜졌다): {message}");
        std::panic::resume_unwind(payload)
    })
}

struct Signal {
    woken: Mutex<bool>,
    ready: Condvar,
}

impl Wake for Signal {
    fn wake(self: Arc<Self>) {
        *self.woken.lock().expect("waker poisoned") = true;
        self.ready.notify_one();
    }
}

/// future 하나를 이 스레드에서 끝까지 돌린다.
///
/// ★런타임 crate 를 안 들이려고 직접 돈다★ — 이 crate 는 워크스페이스 의존 0이고 외부 의존도 최소로
/// 잡았다(lib.rs 불변식 1). 하네스가 도는 future 는 핸들러 하나 또는 가짜 링크 하나뿐이라 스케줄러가
/// 할 일이 없다.
pub fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = Box::pin(future);
    let signal = Arc::new(Signal {
        woken: Mutex::new(false),
        ready: Condvar::new(),
    });
    let waker = Waker::from(Arc::clone(&signal));
    let mut cx = Context::from_waker(&waker);
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(value) => return value,
            Poll::Pending => {
                let mut woken = signal.woken.lock().expect("waker poisoned");
                while !*woken {
                    woken = signal.ready.wait(woken).expect("waker poisoned");
                }
                *woken = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::panic::catch_unwind;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, PoisonError};

    use super::{with_quiet_panic_hook, PANIC_HOOK};

    #[test]
    fn a_panicking_body_unwinds_as_an_ordinary_panic_and_the_previous_hook_comes_back() {
        // abort 가 없다는 단언은 없다 — 되돌리기가 unwind 중으로 돌아가면 이 바이너리가 통째로 죽는다.
        let me = std::thread::current().id();
        let heard = Arc::new(AtomicUsize::new(0));
        let original = {
            let _held = PANIC_HOOK.lock().unwrap_or_else(PoisonError::into_inner);
            let original = Arc::new(std::panic::take_hook());
            let (heard, forward) = (Arc::clone(&heard), Arc::clone(&original));
            std::panic::set_hook(Box::new(move |info| {
                if std::thread::current().id() == me {
                    heard.fetch_add(1, Ordering::SeqCst);
                }
                forward(info);
            }));
            original
        };

        let escaped = catch_unwind(|| with_quiet_panic_hook(|| panic!("body blew up")));

        let payload = escaped.expect_err("본문의 패닉이 밖으로 전파돼야");
        assert_eq!(
            payload.downcast_ref::<&str>(),
            Some(&"body blew up"),
            "페이로드는 원래 패닉 그대로"
        );
        assert_eq!(heard.load(Ordering::SeqCst), 0, "구간 안의 패닉은 조용해야");

        // 탐침은 잠금 안에서 쏜다 — 다른 테스트의 조용한 구간이 겹치면 탐침이 거기 삼켜진다.
        let _held = PANIC_HOOK.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = catch_unwind(|| panic!("probe"));
        let restored = heard.load(Ordering::SeqCst) == 1;
        drop(std::panic::take_hook());
        if let Ok(original) = Arc::try_unwrap(original) {
            std::panic::set_hook(original);
        }
        assert!(restored, "구간이 끝났는데 원래 훅이 돌아오지 않았다");
    }
}
