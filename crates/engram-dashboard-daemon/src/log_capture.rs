//! Log observation harness for tests — ★collects the field text of WARN and ERROR events★.
//!
//! ★Why both levels★: the tests measure not 「which level it was logged at」 but 「whether that event remained / what went into that field」; per-level functions would give one question two harnesses.
//!
//! Entry points: [`capture_loud`] (sync body) · [`capture_loud_async`] (future body) · [`capture_levels`] (`≤DEBUG` + level — the second branch of 「Coverage」).
//!
//! ## ★Coverage — this is not 「the crate's one log observation harness」★
//!
//! This module has two branches — WARN|ERROR field text without level ([`capture_loud`]) · `≤DEBUG` with level ([`capture_levels`]). Two hand-written subscribers remain, each with a different level axis, so they cannot switch over as they are:
//!
//! - `capture_info` in `command_roster` — INFO only (other levels would blur 「that one line」).
//! - `WarnCollector` in `control::mod` — WARN only (ERROR would blur 「did that WARN occur」); the only copy inside a test function.
//!
//! ★Merging would need the level predicate and return shape as arguments — buying 「one harness for the same question」 by making the harness itself configurable★; that trade was not made. Before adding a branch here, see whether one of the four above will do (a fifth copy is the worst).
//!
//! ## ★Observation conditions — without these, an empty result looks like a failure★
//!
//! `with_default` is thread-local; mind both flaws.
//!
//! ① ★A callsite-interest flaw across threads★ (measured tracing 0.1.44 / tracing-core 0.1.36). The interest cache is not sticky — `with_default` goes through `Dispatch::new`, which calls `callsite::register_dispatch` → `CALLSITES.rebuild_interest` and recomputes the interest of every registered callsite at the head of every capture. ★So 「once hit from outside, that spot dies forever」 is wrong — writing it that way makes a tester remove this wrapper as superstition.★
//!    The real flaw is narrow and crosses threads: while only one dispatcher is live (`Rebuilder::JustOne`), a callsite's first registration asks that thread's default subscriber (`dispatcher::get_default`). If another thread is mid-capture and a thread with no subscriber registers that spot first, `NoSubscriber::register_callsite` gives `Interest::never`, there is no later rebuild, and only that capture comes back empty (the assertion quietly fails).
//!    ★The discipline is one — every test that hits that callsite runs inside a capture★; then no subscriber-less thread exists. When it breaks this way, fix the observation condition, not the observed code.
//! ② ★[`capture_loud_async`] cannot see logs outside its own runtime★: it is a current-thread runtime, so spawned tasks run on the same thread, but a body sent to the `spawn_blocking` pool is on a different thread and its logs do not reach this subscriber. Use a non-log observation surface (return values · counts) for that branch.

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
/// ★`with_default` is installed on this thread only★ — logs from another thread are not caught. The async handling: [`capture_loud_async`].
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
/// ★Build the runtime inside this and wrap the whole body★ (thread constraint above): a current-thread runtime runs spawned tasks on the same thread, so their logs reach this subscriber. This function is not `async` (calling it inside a runtime would nest runtimes).
pub(crate) fn capture_loud_async<F: std::future::Future<Output = ()>>(body: F) -> Vec<String> {
    let (_, lines) = capture_loud(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime")
            .block_on(body);
    });
    lines
}

/// Collects the `≤DEBUG` events a sync body made together with their level.
///
/// ★Why DEBUG is on★: judging 「quiet」 by WARN alone makes something that fell to the debug branch look the same as something that went to no branch. Installed on this thread only, so it does not mix with parallel tests; 「Observation conditions」 ① applies as is.
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
