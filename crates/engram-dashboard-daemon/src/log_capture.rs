//! Log observation harness for tests — ★collects the **field text** of WARN and ERROR events★.
//!
//! ★Why both levels are caught★: what the tests using this measure is not 「어느 레벨로 냈나」 (which level it was logged at) but 「그 사건이
//! 남았나 / 그 필드에 무엇이 갔나」 (whether that event remained / what went into that field). Splitting functions per level would give the same question two harnesses.
//!
//! Entry points: [`capture_loud`] (sync body) · [`capture_loud_async`] (future body) · [`capture_levels`] (`≤DEBUG` +
//! level — the second branch of 「덮는 범위」 below).
//!
//! ## ★Coverage — it is **not** 「이 crate 의 로그 관측 하네스는 여기 하나」 (this crate's one log observation harness is here)★
//!
//! This module absorbed two branches — **WARN|ERROR as field text without distinguishing level** ([`capture_loud`]) · **`≤DEBUG`
//! together with the level** ([`capture_levels`]). Two hand-written subscribers remain, and both have a **different level axis** so they cannot
//! be switched over to these functions as they are:
//!
//! - `capture_info` in `command_roster` — catches INFO **only** (if other levels mix in, the 「그 한 줄」 (that one line) the test measures blurs).
//! - `WarnCollector` in `control::mod` — catches WARN **only** (if ERROR mixes in, the 「그 WARN 이 났나」 (did that WARN occur) the test looks at
//!   blurs). It is the only copy that lives **inside** a test function.
//!
//! ★To merge them, the level predicate and the return shape would have to be opened up as arguments, and that is a trade of buying 「같은 질문에 하네스 하나」 (one harness for the same question) by turning
//! **the harness itself into something configurable**★ — that trade was not made in this round. When adding a new branch here,
//! first see whether one of the four above (two in this module + two remaining copies) will do (a fifth copy is the worst).
//!
//! ## ★Observation conditions — without knowing these, **an empty result looks like a failure**★
//!
//! `with_default` is **thread-local**. You must use this knowing both flaws that arise here.
//!
//! ① **A callsite-interest flaw across threads** (measured on tracing 0.1.44 / tracing-core 0.1.36). It is not that the interest cache
//!    is sticky — `with_default` goes through `Dispatch::new`, which calls
//!    `callsite::register_dispatch` → `CALLSITES.rebuild_interest` and **at the head of every capture** recomputes the interest of every
//!    registered callsite. ★So 「한 번이라도 밖에서 때리면 그 자리가 영영 죽는다」 (once hit from outside, that spot dies forever) is
//!    wrong — writing it that way makes whoever tests it see this wrapper as superstition and remove it.★
//!    The real flaw is narrow and crosses threads: while there is only one live dispatcher
//!    (`Dispatchers::rebuilder` → `Rebuilder::JustOne`) the callsite's **first registration** asks the **that thread's** default subscriber
//!    through `dispatcher::get_default`. So while another thread is in the middle of a capture, **if a thread with no subscriber
//!    registers that spot first**, `NoSubscriber::register_callsite` gives `Interest::never` and
//!    there is no later rebuild, so **only that capture comes back empty-handed** (the assertion quietly fails).
//!    ★So the discipline is one — **every test that hits that callsite runs inside a capture**★. If no test hits it from outside,
//!    the 「구독자 없는 스레드」 (thread with no subscriber) above does not exist at all. When it breaks this way, what to fix is the observation condition,
//!    and the code being observed is fine.
//! ② **[`capture_loud_async`] cannot see logs outside its own runtime.** It is a current-thread runtime so spawned tasks
//!    run on the same thread, but a body sent out to the `spawn_blocking` pool is on a **different thread** so its logs
//!    do not reach this subscriber. To measure that branch use an observation surface that is not logs (return values · counts).

use std::sync::{Arc, Mutex};

struct Collector {
    lines: Arc<Mutex<Vec<String>>>,
}

struct Visit<'a>(&'a mut String);

impl tracing::field::Visit for Visit<'_> {
    fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
        self.0.push_str(&format!("{}={:?} ", f.name(), v));
    }
}

impl tracing::subscriber::Subscriber for Collector {
    fn enabled(&self, m: &tracing::Metadata<'_>) -> bool {
        matches!(*m.level(), tracing::Level::WARN | tracing::Level::ERROR)
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::Id {
        tracing::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::Id, _: &tracing::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut buf = String::new();
        event.record(&mut Visit(&mut buf));
        self.lines.lock().expect("lines poisoned").push(buf);
    }
    fn enter(&self, _: &tracing::Id) {}
    fn exit(&self, _: &tracing::Id) {}
}

/// Collects the loud output a sync body makes.
///
/// ★`with_default` is installed **on this thread only**★ — so if the body logs from another thread it is not caught.
/// How that constraint is handled for an async body is in [`capture_loud_async`].
pub(crate) fn capture_loud<T>(body: impl FnOnce() -> T) -> (T, Vec<String>) {
    let lines: Arc<Mutex<Vec<String>>> = Arc::default();
    let collector = Collector {
        lines: lines.clone(),
    };
    let out = tracing::subscriber::with_default(collector, body);
    let captured = lines.lock().expect("lines poisoned").clone();
    (out, captured)
}

/// Collects the loud output a future body makes.
///
/// ★Build the runtime **inside this** and wrap the whole body★: because of the thread constraint above — with a current-thread runtime
/// spawned tasks also run on the same thread so their logs reach this subscriber. So this function is not `async`
/// (calling it inside a runtime would nest runtimes).
pub(crate) fn capture_loud_async<F: std::future::Future<Output = ()>>(body: F) -> Vec<String> {
    let (_, lines) = capture_loud(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("현재 스레드 런타임")
            .block_on(body);
    });
    lines
}

/// Collects the `≤DEBUG` events a sync body made **together with their level**.
///
/// ★Why it turns on even DEBUG★: if 「조용하다」 (quiet) is decided by looking at WARN only, **something that fell to the debug branch** and **something that
/// went to no branch** look the same. `with_default` is installed on this thread only so it does not mix with parallel tests — but 「관측 조건」 (Observation conditions) ① above
/// applies as is (every test that hits that callsite runs inside a capture).
pub(crate) fn capture_levels(body: impl FnOnce()) -> Vec<(tracing::Level, String)> {
    struct Leveled {
        lines: Arc<Mutex<Vec<(tracing::Level, String)>>>,
    }
    impl tracing::subscriber::Subscriber for Leveled {
        fn enabled(&self, m: &tracing::Metadata<'_>) -> bool {
            *m.level() <= tracing::Level::DEBUG
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::Id {
            tracing::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::Id, _: &tracing::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            let mut buf = String::new();
            event.record(&mut Visit(&mut buf));
            self.lines
                .lock()
                .expect("lines poisoned")
                .push((*event.metadata().level(), buf));
        }
        fn enter(&self, _: &tracing::Id) {}
        fn exit(&self, _: &tracing::Id) {}
    }

    let lines: Arc<Mutex<Vec<(tracing::Level, String)>>> = Arc::default();
    tracing::subscriber::with_default(
        Leveled {
            lines: lines.clone(),
        },
        body,
    );
    let captured = lines.lock().expect("lines poisoned").clone();
    captured
}
