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
/// ★되돌리기는 [`Drop`] 이 한다★ — `body` 가 unwind 하면 대입문까지 못 가서 **조용한 훅이 프로세스에
/// 남고**, 그 뒤의 모든 테스트가 진짜 패닉의 출력을 잃는다(패닉을 일부러 태우는 하네스라 실제로 닿는다).
pub fn with_quiet_panic_hook<T>(body: impl FnOnce() -> T) -> T {
    let _held = PANIC_HOOK.lock().unwrap_or_else(PoisonError::into_inner);
    let _restore = RestoreHook(Some(std::panic::take_hook()));
    std::panic::set_hook(Box::new(|_| {}));
    body()
}

type PanicHook = Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Sync + Send + 'static>;

/// 잡아 둔 훅을 되돌린다. 잠금(`_held`)보다 **나중에** 선언해야 훅 교체가 잠금 안에서 끝난다.
struct RestoreHook(Option<PanicHook>);

impl Drop for RestoreHook {
    fn drop(&mut self) {
        if let Some(previous) = self.0.take() {
            std::panic::set_hook(previous);
        }
    }
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
