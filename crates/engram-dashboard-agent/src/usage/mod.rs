//! The vendor-neutral vocabulary of usage-limit observations — made by backends, received by the daemon
//!   ([`crate::types::StatusSink::usage_observed`]).
//!
//! ★No vendor names · policy (cooldown · deadline) here★ — each vendor's backend module carries them (ADR-0004 「backend extension」); a
//! vendor is one opaque word here. Shared producer-side tools live here too — field normalization ([`used_pct_from_fraction`] etc.) ·
//! upstream string tools ([`display_text`]·[`has_word`]) · abnormal-result grounds and upstream original text
//! ([`UsageDetail`]·[`UpstreamText`]) · the decoder wrapper blocking non-default-account pickup ([`UsageGate`]). The active-query seam
//!   ([`UsageProbe`]·[`ProbeSpawner`]) and its real implementations ([`OsProbeSpawner`]·[`ScratchDir`]) are here too.
// ADR-0004

mod detail;
mod gate;
mod normalize;
mod probe;
mod process;
mod scratch;
#[cfg(test)]
pub(crate) mod testing;
mod text;

pub use detail::{UpstreamText, UsageDetail};
pub use gate::{env_overrides_daemon, UsageGate};
pub use normalize::{
    resets_at_from_epoch_secs, resets_at_from_json, used_pct_from_fraction, used_pct_from_percent,
};
pub use probe::{
    finish_after_answer, ExitInfo, ProbeChild, ProbeCommand, ProbeEnv, ProbeError, ProbeFailure,
    ProbeSpawner, UsagePolicy, UsageProbe,
};
pub use process::{OsProbeSpawner, KILL_WAIT};
pub use scratch::{sweep_stale_scratch, ScratchDir};
pub use text::{display_text, has_word};

/// An opaque word that points at a vendor — the receiving side does not interpret it, only uses it as an entry key.
///
/// ★The canonical spelling is the single `&'static str` the vendor declared★ — incoming words (wire lowercase · bus capitalized) are
///   folded to it before use as a key; an unfolded key makes the two entrances hit different entries.
/// ★Only `&'static str` is accepted, on purpose — do not widen to `String`★: a string from disk · wire cannot make a key (unless leaked),
///   so it must go through the canonical form. Literals cannot be blocked — only vendor declarations and test doubles should call `new`
///   (the daemon doing so violates ADR-0004).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UsageVendorKey(&'static str);

impl UsageVendorKey {
    pub const fn new(word: &'static str) -> Self {
        Self(word)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

/// A word telling accounts apart within one vendor. ★For now always the [`Default`] value (`"default"` — the daemon env's default login)★
/// — the field is laid down early so a client attached to several daemons can tell where a value came from. It goes over the wire · to disk.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UsageAccountKey(String);

impl UsageAccountKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for UsageAccountKey {
    fn default() -> Self {
        Self("default".to_owned())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UsageKey {
    pub vendor: UsageVendorKey,
    pub account: UsageAccountKey,
}

/// An observation of one limit window.
///
/// - `used_pct` = percentage **used** (not remaining); the producer normalizes it to a finite 0–100.
/// - `resets_at` = reset time, epoch seconds.
/// - `None` in both fields = unknown, not 0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowObs {
    pub used_pct: Option<f64>,
    pub resets_at: Option<i64>,
}

/// One per-model weekly window. `label` = the display name the vendor gave (not used as a key).
#[derive(Debug, Clone, PartialEq)]
pub struct ScopedWindowObs {
    pub label: String,
    pub window: WindowObs,
}

/// Where the observation came from. What a missing field means is unrelated to this ([`UsageObservation`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageSource {
    Passive,
    Active,
}

/// One usage observation of one vendor.
///
/// ★A missing field (`None`) means 「not carried」 whatever the source★ — the receiving side keeps the value it held (same for
///   `model_scoped` · `plan`); a null window in a query result is not 「none」 either.
/// `model_scoped: Some` replaces the whole list, not per item; `Some(vec![])` = 「no per-model window」 — the receiver empties its list
///   (emitted when the producer read the container and it had 0 items).
/// ★The observation time is not carried★ — the receiving daemon stamps it; the producer (decoder) has no clock.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageObservation {
    pub vendor: UsageVendorKey,
    pub five_hour: Option<WindowObs>,
    pub weekly: Option<WindowObs>,
    pub model_scoped: Option<Vec<ScopedWindowObs>>,
    pub plan: Option<String>,
    pub source: UsageSource,
    /// `Some` = the query succeeded but upstream answered 「this account has no limit information」 — the value is its grounds (Claude
    /// `rate_limits_available: false` — API key · Bedrock · Vertex · a token without profile permission · logged out).
    /// If `Some`, the window fields (`five_hour` · `weekly` · `model_scoped`) are all `None` and `source` is `Active` — a promise kept by
    /// the maker. The receiver empties all window values and folds into 「no information」 — not a failure (a failure keeps held values). A
    ///   `Passive` observation is always `None`.
    pub limits_unavailable: Option<UsageDetail>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AgentId, AgentInfo, AgentStatus, StatusSink};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn observation() -> UsageObservation {
        UsageObservation {
            vendor: UsageVendorKey::new("test-vendor"),
            five_hour: Some(WindowObs {
                used_pct: Some(40.0),
                resets_at: Some(1_900_000_000),
            }),
            weekly: None,
            model_scoped: None,
            plan: None,
            source: UsageSource::Passive,
            limits_unavailable: None,
        }
    }

    /// A sink that implements only the required hooks — every existing sink that does not use the new hooks has this shape.
    struct RequiredOnly {
        calls: AtomicUsize,
    }

    impl StatusSink for RequiredOnly {
        fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {
            self.calls.fetch_add(1, Ordering::SeqCst);
        }
        fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {
            self.calls.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn usage_observed_defaults_to_a_no_op() {
        let sink = RequiredOnly {
            calls: AtomicUsize::new(0),
        };
        sink.usage_observed(observation());
        assert_eq!(
            sink.calls.load(Ordering::SeqCst),
            0,
            "the default implementation must not leak into another hook"
        );
    }

    /// The account word goes out over the wire (`account_key`) and to disk — if changed, it no longer matches stored entries.
    #[test]
    fn the_default_account_word_is_default() {
        assert_eq!(UsageAccountKey::default().as_str(), "default");
    }

    /// The daemon's subscription set (`BTreeSet`) iterates in this order — string order, not the word's address, so one word is one element.
    #[test]
    fn vendor_keys_order_as_their_words() {
        let mut keys = vec![
            UsageVendorKey::new("b-vendor"),
            UsageVendorKey::new("a-vendor-2"),
            UsageVendorKey::new("a-vendor"),
        ];
        keys.sort();
        assert_eq!(
            keys.iter().map(|k| k.as_str()).collect::<Vec<_>>(),
            ["a-vendor", "a-vendor-2", "b-vendor"]
        );
        let leaked: &'static str = Box::leak(String::from("a-vendor").into_boxed_str());
        assert_eq!(
            UsageVendorKey::new(leaked).cmp(&UsageVendorKey::new("a-vendor")),
            std::cmp::Ordering::Equal,
            "the same word at a different address is the same key"
        );
    }
}
