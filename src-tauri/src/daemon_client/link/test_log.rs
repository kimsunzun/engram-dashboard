//! A tracing subscriber that records this thread's events as lines — for tests that pin what is logged.

use std::sync::{Arc, Mutex};

/// Runs `f` with [`Lines`] as this thread's subscriber.
///
/// ★Callsite interest is a process-wide cache★ (tracing-core 0.1.36 `callsite.rs`): while a `with_default`
/// subscriber is the only registered dispatcher, a callsite another test thread hits first is registered through
/// that thread's own default (none) and cached as "never", so this thread's event at that callsite is lost (seen:
/// one failure in a few full `lib_unit` runs). A second dispatcher kept alive for the whole process makes every
/// registration and rebuild consult all live dispatchers, this one included; rebuilding just before `f` closes the
/// window in which the pin itself was the only one.
pub(crate) fn capture<R>(lines: Arc<Mutex<Vec<String>>>, f: impl FnOnce() -> R) -> R {
    static PIN: std::sync::OnceLock<tracing::Dispatch> = std::sync::OnceLock::new();
    PIN.get_or_init(|| tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default()));
    tracing::subscriber::with_default(Lines(lines), || {
        tracing::callsite::rebuild_interest_cache();
        f()
    })
}

/// Every event on this thread as one line: `level=<LEVEL> ` and then every field as `name=value `.
struct Lines(Arc<Mutex<Vec<String>>>);

struct Fields<'a>(&'a mut String);

impl tracing::field::Visit for Fields<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.push_str(&format!("{}={value:?} ", field.name()));
    }
}

impl tracing::Subscriber for Lines {
    fn register_callsite(
        &self,
        _: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        tracing::subscriber::Interest::sometimes()
    }
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut line = format!("level={} ", event.metadata().level());
        event.record(&mut Fields(&mut line));
        self.0.lock().unwrap().push(line);
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}
