//! 패닉 가둠 표시와 가두지 못한 패닉의 훅 몸통(ADR-0292).
//!
//! 가두는 자리는 [`contain`] 으로 감싸고, 그 안에서 여러 에이전트가 함께 쓰는 상태를 고치는 호출은
//! [`uncontained`] 로 감싼다. 둘을 가르는 것은 스레드 지역 깊이 하나다 — 훅 몸통([`hook`])은 늘 사유를
//! 로그하고 이전 훅을 부른 뒤, 깊이가 0 이면 끝내기(운영 = `std::process::abort` — [`install_hook`])를 부르고
//! 아니면 돌아가 되감기가 그 [`contain`] 까지 가서 `Err` 로 접힌다. 가둔 패닉의 자리(파일 · 줄)가 남는 곳은 그
//! 사유 줄뿐이다.
//!
//! ★표시가 있어야 하는 까닭★ — std 의 훅은 패닉한 스레드에서 **되감기 전에** 돈다. 표시 없이 「잡히지 않을
//! 패닉 = 끝」 훅을 걸면 `catch_unwind` 가 잡으려던 패닉까지 끝낸다.
//!
//! ★불변식 — 표시 범위 ⊆ 그것을 잡는 catch 클로저★: 깊이를 올리는 길은 [`contain`] 하나이고, 거기서 올림과
//! 잡음은 한 몸이다. 깊이만 따로 올리는 공개 수단을 두지 않는다 — 잡는 이 없이 표시가 서면 그 안의 패닉이 훅을
//! 지나 스레드만 죽인다(펌프라면 끝 전이 없이 `Running` 으로 남는 에이전트와 산 CLI — ADR-0005 · ADR-0290).
//!
//! 표시는 스레드를 따라가지 않는다 — 감싼 코드가 띄운 스레드 · 태스크는 표시 밖(= 끝냄)이다. 비동기 표시도
//! 없다: 가두는 자리가 전부 동기이고, async 자리를 가두려면 태스크 지역 표시가 따로 있어야 한다.
//!
//! 로그는 `tracing` facade 를 바로 부른다 — 이 crate 의 `logging` 입주자를 부르지 않는다(lib.rs 게이트 ③).
//! 쥔 채 패닉한 스레드가 로그 파일에 다시 들어가는 자기교착과, 남이 쥔 채 멎은 파일 잠금 앞의 끝없는 대기는 그
//! 입주자의 파일 sink 가 막는다(그 줄을 버린다). `tracing` 내부 잠금 앞의 대기는 막지 않는다.
// ADR-0292

use std::any::Any;
use std::cell::Cell;
use std::fmt;
use std::panic::{catch_unwind, PanicHookInfo, UnwindSafe};
use std::sync::Once;

thread_local! {
    // const 초기화 + `Drop` 없는 값이라 스레드 해체 중(TLS 소멸자 안)에도 읽힌다고 보지만, 이는 std 구현의
    //   동작이지 문서화된 보장이 아니다. 못 읽으면 「표시 없음」 = 끝내는 쪽으로 읽는다.
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}

const NON_STRING_PAYLOAD: &str = "<non-string panic payload>";

/// 이 스레드가 지금 [`contain`] 안이고 그 안의 [`uncontained`] 밖인가. 못 읽으면 `false`.
pub fn is_contained() -> bool {
    DEPTH.try_with(|d| d.get() > 0).unwrap_or(false)
}

/// 깊이를 바꾸고, drop 에서 바꾸기 전 값으로 되돌린다 — 되감기로 빠져나가도 되돌아간다.
///
/// ★drop 은 패닉하지 않는다★ — 되감기 중의 소멸자 패닉은 abort 다. 그래서 `try_with` 만 쓴다.
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

/// [`contain`] 이 잡은 패닉 — 페이로드를 [`payload_text`] 로 옮긴 문장만 나른다.
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

/// `f` 를 표시 안에서 돌리고 그 패닉을 잡는다 — 잡은 패닉 = `Err`.
///
/// 표시는 `f` 가 끝나거나 되감겨 나올 때 들어오기 전 값으로 돌아간다. 사유 줄은 훅이 표시와 무관하게 남기므로
/// 부르는 쪽은 다시 로그하지 않고 실패 표면(`Failed` · 입력 거절)만 낸다 — 단 훅 줄에 없는
/// 귀속(에이전트 id 등)은 남긴다.
/// 잡은 페이로드는 이 표시가 내려간 뒤 버려진다 — 그 drop 의 패닉은 바깥 표시로 판정된다.
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

/// `f` 를 표시 밖에서 돌린다 — 그 안의 패닉은 바깥에 [`contain`] 이 있어도 훅이 끝낸다. 끝나면(되감겨 나와도)
/// 표시가 들어오기 전 값으로 돌아온다. 그 안에서 다시 [`contain`] 을 부르면 그 범위는 다시 가둬진다.
///
/// 가둔 코드가 여러 에이전트의 공유 상태를 고치는 호출을 감싼다 — 반쯤 고친 공유 상태를 남긴 채 이어 가지
/// 않게(ADR-0292 결정 3).
// ADR-0292
pub fn uncontained<T>(f: impl FnOnce() -> T) -> T {
    let _lowered = Depth::change(|_| 0);
    f()
}

/// 패닉 페이로드 → 문장. `&str` · `String` 이 아니면 `"<non-string panic payload>"`.
///
/// `catch_unwind` 의 `Box<dyn Any + Send>` 를 `&payload` 로 넘겨도 컴파일된다(상자 자체가 `Any` 라 말없이
/// 바뀐다) — 그 한 겹은 여기서 벗긴다.
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

/// 훅 몸통 — 사유 줄을 로그하고 이전 훅을 부른 뒤, 표시 밖이면 `end` 를 부른다.
///
/// 판정은 들어온 때의 표시로 한다. 표시 안이면 돌아가고 되감기가 그 [`contain`] 까지 간다.
/// 로그는 표시와 무관하다 — 패닉 자리(파일 · 줄)는 훅에서만 보이므로, 잡고 로그하지 않는 자리의 기록도 여기
/// 남는다. `end` 가 돌아오면(시험) 이 함수도 돌아가고 되감기가 이어진다.
/// ★여기서 난 패닉(이전 훅 안 포함)은 표시와 무관하게 std 가 곧장 abort 한다★ — 훅 안의 패닉은 되감기지 않는다.
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

/// [`hook`] 을 프로세스 훅으로 건다 — 끝내기 = `std::process::abort`, 이전 훅 = 걸 때 있던 훅(std 기본이면
/// stderr 출력). 멱등이다(두 번째부터 아무것도 안 한다).
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

    /// 훅은 프로세스 전역이다 — 갈아 끼우는 시험끼리 겹치지 않게 한 자물쇠로 줄 세운다.
    static HOOK: Mutex<()> = Mutex::new(());

    #[derive(Default)]
    struct Heard {
        previous: AtomicUsize,
        ended: AtomicUsize,
    }

    /// 이 스레드의 패닉만 [`hook`] 에 넘기는 훅을 걸고 `body` 를 돌린다 — 이전 훅 · 끝내기는 세기만 한다.
    /// 다른 스레드(같은 바이너리의 다른 시험)의 패닉은 원래 훅으로 넘긴다.
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
        // ★되돌리기를 `Drop` 으로 옮기지 말 것★ — std 는 패닉 중인 스레드의 `set_hook` 을 거부하고, 되감기 중
        //   소멸자 안의 그 패닉은 abort 다(command `with_quiet_panic_hook` 과 같은 까닭). 잡은 뒤에 되돌린다.
        let outcome = catch_unwind(AssertUnwindSafe(body));
        drop(std::panic::take_hook());
        match Arc::try_unwrap(original) {
            Ok(original) => std::panic::set_hook(original),
            Err(shared) => std::panic::set_hook(Box::new(move |info| shared(info))),
        }
        (outcome, heard)
    }

    // ── 표시 깊이 ──

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

    // ── 표시 끔 ──

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

    // ── 훅 판정 ──

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

    /// 데몬 훅이 남기던 사유 줄(스레드 · 문장 · 자리)과 같은 꼴 — 표시 안에서도 남긴다.
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

    // ── 페이로드 문장 ──

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
