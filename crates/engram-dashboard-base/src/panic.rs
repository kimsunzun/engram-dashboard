//! Panic containment mark and the hook body for panics that were not contained (ADR-0292).
//!
//! A containing site is wrapped in [`contain`], and inside it a call that fixes state shared by several agents is
//! wrapped in [`uncontained`]. What tells the two apart is one thread-local depth — the hook body ([`hook`]) always
//! logs the reason and calls the previous hook, then if the depth is 0 calls the end (production = `std::process::abort` — [`install_hook`]),
//! otherwise returns, and unwinding goes up to that [`contain`] and is folded into `Err`. The only place where the location (file · line) of a contained panic remains is that
//! reason line.
//!
//! ★Why the mark is needed★ — std's hook runs on the panicking thread **before unwinding**. If a 「잡히지 않을
//! 패닉 = 끝」 (an uncaught panic = end) hook is installed without a mark, it ends even the panics that `catch_unwind` meant to catch.
//!
//! ★Invariant — mark range ⊆ the catch closure that catches it★: the only path that raises the depth is [`contain`], and there raising and
//! catching are one body. No public means to raise only the depth is provided — if a mark stands with no one to catch, a panic inside it passes the hook
//! and kills only the thread (for a pump, an agent left as `Running` with no terminal transition, and a live CLI — ADR-0005 · ADR-0290).
//!
//! The mark does not follow threads — a thread or task launched by the wrapped code is outside the mark (= ends). There is no async mark either:
//! all containing sites are synchronous, and containing an async site would need a separate task-local mark.
//!
//! Logging calls the `tracing` facade directly — it does not call this crate's `logging` tenant (lib.rs gate ③).
//! The self-deadlock of a thread that panicked while holding it re-entering the log file, and endless waiting at a file lock held and stuck by another, are
//! blocked by that tenant's file sink (it drops the line). Waiting at the `tracing` internal lock is not blocked.
// ADR-0292

use std::any::Any;
use std::cell::Cell;
use std::fmt;
use std::panic::{catch_unwind, PanicHookInfo, UnwindSafe};
use std::sync::Once;

thread_local! {
    // Const-initialized and a value without `Drop`, so it is thought to be readable even during thread teardown (inside a TLS destructor), but that is std implementation
    //   behavior, not a documented guarantee. If it cannot be read, it is read as 「표시 없음」 (no mark) = the ending side.
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}

const NON_STRING_PAYLOAD: &str = "<non-string panic payload>";

/// Is this thread now inside [`contain`] and outside its [`uncontained`]? `false` if it cannot be read.
pub fn is_contained() -> bool {
    DEPTH.try_with(|d| d.get() > 0).unwrap_or(false)
}

/// Changes the depth, and on drop restores the value from before the change — it is restored even when leaving by unwinding.
///
/// ★drop does not panic★ — a destructor panic during unwinding is abort. So only `try_with` is used.
struct Depth {
    before: Option<u32>,
}

impl Depth {
    fn change(to: impl FnOnce(u32) -> u32) -> Self {
        let before = DEPTH
            .try_with(|d| {
                let before = d.get();
                d.set(to(before));
                before
            })
            .ok();
        Self { before }
    }
}

impl Drop for Depth {
    fn drop(&mut self) {
        if let Some(before) = self.before {
            let _ = DEPTH.try_with(|d| d.set(before));
        }
    }
}

/// A panic caught by [`contain`] — carries only the sentence into which the payload was turned by [`payload_text`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caught {
    message: String,
}

impl Caught {
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Caught {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// Runs `f` inside the mark and catches its panic — a caught panic = `Err`.
///
/// The mark returns to its pre-entry value when `f` ends or unwinds out. The hook leaves the reason line regardless of the mark, so
/// the caller does not log again and only emits the failure surface (`Failed` · input rejection) — but it does keep an
/// attribution not in the hook line (agent id etc.).
/// The caught payload is dropped after this mark is lowered — a panic in that drop is decided by the outer mark.
// ADR-0292
pub fn contain<T>(f: impl FnOnce() -> T + UnwindSafe) -> Result<T, Caught> {
    catch_unwind(move || {
        let _raised = Depth::change(|d| d.saturating_add(1));
        f()
    })
    .map_err(|payload| Caught {
        message: payload_text(&*payload),
    })
}

/// Runs `f` outside the mark — a panic inside it ends via the hook even if there is a [`contain`] outside. When it finishes (even by unwinding out),
/// the mark returns to its pre-entry value. If [`contain`] is called again inside it, that range is contained again.
///
/// Contained code wraps with this a call that fixes state shared by several agents — so as not to carry on leaving half-fixed shared state
/// (ADR-0292 decision 3).
// ADR-0292
pub fn uncontained<T>(f: impl FnOnce() -> T) -> T {
    let _lowered = Depth::change(|_| 0);
    f()
}

/// Panic payload → sentence. If it is not `&str` · `String`, `"<non-string panic payload>"`.
///
/// Passing the `Box<dyn Any + Send>` of `catch_unwind` as `&payload` also compiles (the box itself is `Any`, so it silently
/// changes) — that one layer is peeled off here.
pub fn payload_text(payload: &(dyn Any + Send)) -> String {
    if let Some(inner) = payload.downcast_ref::<Box<dyn Any + Send>>() {
        return payload_text(&**inner);
    }
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| NON_STRING_PAYLOAD.to_string())
}

/// Hook body — logs the reason line and calls the previous hook, then calls `end` if outside the mark.
///
/// The decision uses the mark at the time of entry. If inside the mark it returns and unwinding goes up to that [`contain`].
/// Logging is independent of the mark — the panic location (file · line) is visible only in the hook, so a record of a site that catches and does not log also
/// remains here. If `end` returns (tests), this function returns too and unwinding continues.
/// ★A panic that occurs here (including inside the previous hook) is aborted by std at once regardless of the mark★ — a panic inside a hook does not unwind.
// ADR-0292
pub fn hook(info: &PanicHookInfo<'_>, previous: &dyn Fn(&PanicHookInfo<'_>), end: &dyn Fn()) {
    let contained = is_contained();
    let thread = std::thread::current();
    let name = thread.name().unwrap_or("<unnamed>");
    let msg = payload_text(info.payload());
    let location = info
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_else(|| "<unknown>".to_string());
    tracing::error!(thread = name, location, "스레드 panic: {msg}");
    previous(info);
    if !contained {
        end();
    }
}

/// Installs [`hook`] as the process hook — end = `std::process::abort`, previous hook = the hook present at install time (if it is the std default,
/// stderr output). Idempotent (from the second call it does nothing).
// ADR-0292
pub fn install_hook() {
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| hook(info, &*previous, &end_process)));
    });
}

fn end_process() {
    std::process::abort()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
    use std::sync::{Arc, Mutex, PoisonError};

    fn depth() -> u32 {
        DEPTH.with(Cell::get)
    }

    /// The hook is process-global — tests that swap it are lined up behind one lock so they do not overlap.
    static HOOK: Mutex<()> = Mutex::new(());

    #[derive(Default)]
    struct Heard {
        previous: AtomicUsize,
        ended: AtomicUsize,
    }

    /// Installs a hook that passes only this thread's panics to [`hook`] and runs `body` — the previous hook · end only count.
    /// Panics of other threads (other tests of the same binary) are passed to the original hook.
    fn under_hook<T>(body: impl FnOnce() -> T) -> (std::thread::Result<T>, Arc<Heard>) {
        let _held = HOOK.lock().unwrap_or_else(PoisonError::into_inner);
        let me = std::thread::current().id();
        let heard = Arc::new(Heard::default());
        let original = Arc::new(std::panic::take_hook());
        {
            let (heard, original) = (Arc::clone(&heard), Arc::clone(&original));
            std::panic::set_hook(Box::new(move |info| {
                if std::thread::current().id() != me {
                    return original(info);
                }
                hook(
                    info,
                    &|_| {
                        heard.previous.fetch_add(1, SeqCst);
                    },
                    &|| {
                        heard.ended.fetch_add(1, SeqCst);
                    },
                );
            }));
        }
        // ★Do not move the restore into `Drop`★ — std refuses `set_hook` on a panicking thread, and that panic inside a destructor
        //   during unwinding is abort (same reason as command `with_quiet_panic_hook`). Restore after catching.
        let outcome = catch_unwind(AssertUnwindSafe(body));
        drop(std::panic::take_hook());
        match Arc::try_unwrap(original) {
            Ok(original) => std::panic::set_hook(original),
            Err(shared) => std::panic::set_hook(Box::new(move |info| shared(info))),
        }
        (outcome, heard)
    }

    // ── Mark depth ──

    #[test]
    fn contain_raises_the_mark_only_inside_and_nests() {
        assert_eq!(depth(), 0);
        let seen = contain(|| {
            let nested = contain(depth).expect("패닉 없음");
            (is_contained(), depth(), nested)
        })
        .expect("패닉 없음");
        assert_eq!(seen, (true, 1, 2));
        assert_eq!(depth(), 0);
        assert!(!is_contained());
    }

    #[test]
    fn contain_hands_back_the_value_or_the_panic_text() {
        let (outcome, heard) = under_hook(|| {
            (
                contain(|| 7),
                contain(|| -> u32 { panic!("고정 문장") }),
                contain(|| -> u32 { panic!("값 {}", 3) }),
            )
        });
        let (value, fixed, formatted) = outcome.expect("contain 이 잡아야");
        assert_eq!(value, Ok(7));
        assert_eq!(fixed.expect_err("패닉").message(), "고정 문장");
        assert_eq!(formatted.expect_err("패닉").to_string(), "값 3");
        assert_eq!(heard.ended.load(SeqCst), 0, "가둔 패닉은 끝내지 않는다");
    }

    #[test]
    fn the_mark_comes_back_after_an_unwind_through_contain() {
        let (outcome, _) = under_hook(|| {
            contain(|| {
                let inner = contain(|| -> u32 { panic!("안쪽") });
                (inner.is_err(), depth())
            })
        });
        assert_eq!(outcome.expect("잡혀야"), Ok((true, 1)));
        assert_eq!(depth(), 0);
    }

    // ── Mark off ──

    #[test]
    fn uncontained_lowers_the_mark_to_zero_and_restores_it() {
        let seen = contain(|| {
            let inside = uncontained(|| (depth(), is_contained()));
            (inside, depth())
        })
        .expect("패닉 없음");
        assert_eq!(seen, ((0, false), 1));
        assert_eq!(uncontained(depth), 0);
        assert_eq!(depth(), 0, "표시 밖에서 부르면 되돌린 값도 0");
    }

    #[test]
    fn uncontained_restores_the_mark_when_it_unwinds() {
        let (outcome, _) = under_hook(|| {
            contain(|| {
                let unwound = catch_unwind(|| uncontained(|| -> u32 { panic!("표시 끔 안") }));
                (unwound.is_err(), depth())
            })
        });
        assert_eq!(outcome.expect("잡혀야"), Ok((true, 1)));
        assert_eq!(depth(), 0);
    }

    #[test]
    fn contain_inside_uncontained_marks_again() {
        let seen = contain(|| uncontained(|| contain(depth)))
            .expect("패닉 없음")
            .expect("패닉 없음");
        assert_eq!(seen, 1);
        assert_eq!(depth(), 0);
    }

    // ── Hook decision ──

    #[test]
    fn a_panic_inside_contain_does_not_end() {
        let (outcome, heard) = under_hook(|| contain(|| -> u32 { panic!("가둔 패닉") }));
        assert!(outcome.expect("contain 이 잡는다").is_err());
        assert_eq!(heard.ended.load(SeqCst), 0);
        assert_eq!(
            heard.previous.load(SeqCst),
            1,
            "이전 훅은 표시와 무관하게 부른다"
        );
    }

    #[test]
    fn a_panic_outside_any_mark_ends() {
        let (outcome, heard) = under_hook(|| -> u32 { panic!("가두지 않은 패닉") });
        assert!(
            outcome.is_err(),
            "시험의 끝내기는 돌아오므로 되감기가 이어진다"
        );
        assert_eq!(heard.ended.load(SeqCst), 1);
        assert_eq!(heard.previous.load(SeqCst), 1);
    }

    #[test]
    fn a_panic_under_uncontained_ends_even_inside_contain() {
        let (outcome, heard) =
            under_hook(|| contain(|| uncontained(|| -> u32 { panic!("표시 끔 안 패닉") })));
        assert!(outcome.expect("바깥 contain 이 되감기를 받는다").is_err());
        assert_eq!(heard.ended.load(SeqCst), 1);
    }

    #[test]
    fn a_panic_in_contain_under_uncontained_does_not_end() {
        let (outcome, heard) = under_hook(|| {
            contain(|| uncontained(|| contain(|| -> u32 { panic!("다시 가둔 패닉") })))
        });
        assert!(matches!(outcome, Ok(Ok(Err(_)))), "{outcome:?}");
        assert_eq!(heard.ended.load(SeqCst), 0);
    }

    struct Collected(Arc<Mutex<Vec<u8>>>);

    impl io::Write for Collected {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// The same shape as the reason line the daemon hook used to leave (thread · sentence · location) — it is left even inside the mark.
    #[test]
    fn the_hook_logs_the_reason_line_even_inside_contain() {
        let written = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer({
                let written = Arc::clone(&written);
                move || Collected(Arc::clone(&written))
            })
            .finish();

        let (outcome, _) = under_hook(|| {
            tracing::subscriber::with_default(subscriber, || {
                contain(|| -> u32 { panic!("사유 {}", 42) })
            })
        });

        assert!(outcome.expect("잡혀야").is_err());
        let text = String::from_utf8(
            written
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .expect("utf-8");
        assert!(text.contains("ERROR"), "{text}");
        assert!(text.contains("스레드 panic: 사유 42"), "{text}");
        assert!(text.contains("thread="), "{text}");
        assert!(text.contains("panic.rs:"), "패닉 자리: {text}");
    }

    // ── Payload sentence ──

    #[test]
    fn payload_text_reads_str_and_string_and_names_the_rest() {
        assert_eq!(payload_text(&"고정"), "고정");
        assert_eq!(payload_text(&String::from("만든")), "만든");
        assert_eq!(payload_text(&42_u32), NON_STRING_PAYLOAD);
        let boxed: Box<dyn Any + Send> = Box::new("상자 안");
        assert_eq!(payload_text(&*boxed), "상자 안");
        assert_eq!(payload_text(&boxed), "상자 안", "한 겹 더 싼 상자도 벗긴다");
    }
}
