//! Harness only — lives behind the feature flag `test-support` (ADR-0012).

use std::future::Future;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::task::{Context, Poll, Wake, Waker};

/// The panic hook is **process-global**, so the spans that grab and restore it must not overlap.
static PANIC_HOOK: Mutex<()> = Mutex::new(());

/// Turns off the default output of an intended panic and runs `body` — when done, it restores the original hook.
///
/// ★The tests of one test binary run **concurrently** on threads★ — if each swaps the hook, the one that finishes later
/// restores another's hook thinking it is the original, or another's panic output gets mixed into a span that should be quiet. This race is
/// blocked at this one place (the lock covers the whole hook-swap span).
/// ★Inside the span, panic output from **unrelated threads** (other tests of the same binary) also disappears★ — because the hook is
/// process-global. So the span is kept narrow to cover only the moment the intended panic occurs. Not silencing by thread is
/// intentional — there are callers where the intended panic occurs on a thread the body launched (the codex writer · tokio blocking
/// pool).
/// ★A panic that leaks out of `body` is not an intended one★ — we revive its message, print it, and rethrow with **the same payload**.
/// To the caller it looks like an ordinary test failure.
pub fn with_quiet_panic_hook<T>(body: impl FnOnce() -> T) -> T {
    let _held = PANIC_HOOK.lock().unwrap_or_else(PoisonError::into_inner);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    // ★Do not move the restore into `Drop`★ — std refuses `set_hook` on a panicking thread, and that refusal becomes
    //   a second panic inside a destructor during unwind, so the process aborts (the test binary dies whole with no
    //   message — Windows `0xC0000409`, measured). But with only an assignment statement, unwind skips that line and
    //   a quiet hook remains. So we restore **after** catching the unwind.
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

/// Runs one future to completion on this thread.
///
/// ★It runs directly so as not to bring in a runtime crate★ — this crate has zero workspace dependencies and keeps external dependencies minimal
/// too (lib.rs invariant 1). The futures the harness runs are only one handler or one fake link, so the scheduler has
/// nothing to do.
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
        // There is no assertion that there is no abort — if the restore goes back to being during unwind, this binary dies whole.
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

        // The probe is fired inside the lock — if another test's quiet span overlaps, the probe is swallowed there.
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
