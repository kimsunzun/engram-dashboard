//! The vendor-neutral vocabulary of usage-limit observations — made by backends and received by the daemon ([`crate::types::StatusSink::usage_observed`]).
//!
//! ★Do not put vendor names · policy (cooldown · deadline) here★ — each vendor's backend module carries them (ADR-0004
//!   「백엔드 확장」). This module knows a vendor only as one opaque word.
//! Shared producer-side tools live here too — field value normalization ([`used_pct_from_fraction`] etc.) · upstream string tools
//! ([`display_text`]·[`has_word`]) · the grounds of an abnormal result and the upstream original text ([`UsageDetail`]·[`UpstreamText`]) · the decoder wrapper that blocks the pickup of agents that are not the
//! default account ([`UsageGate`]).
//! The seam of active query ([`UsageProbe`]·[`ProbeSpawner`]) and its real implementations ([`OsProbeSpawner`]·[`ScratchDir`]) are here too —
//! each backend implements the vendor query on top of this seam.
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

/// An opaque word that points at a vendor — the receiving side does not interpret it and uses it only as an entry key.
///
/// ★The canonical spelling is the single `&'static str` the vendor declared★ — incoming words (wire lowercase · bus capitalized)
///   are folded to that canonical form before being used as the key. A key made from an unfolded word makes the two entrances hit different entries.
/// ★Accepting only `&'static str` is intended★: a string from disk · wire cannot make a key (unless leaked), so it has to go through the canonical form.
///   Do not widen it to `String`. But literals cannot be blocked — only vendor declarations and test doubles should call `new`
///   (the daemon calling it with a vendor literal violates ADR-0004).
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

/// A word that tells accounts apart within the same vendor. ★For now it is always the [`Default`] value (`"default"` — the daemon env's default login)
/// ★ — the field is laid down early so that a client attached to several daemons can tell where a value came from.
/// That word goes out over the wire · to disk.
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

/// The key of one usage-state entry — `(벤더, 계정)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UsageKey {
    pub vendor: UsageVendorKey,
    pub account: UsageAccountKey,
}

/// An observation of one limit window.
///
/// - `used_pct` = the percentage of the amount **used** (not the amount remaining). The producer normalizes it to a finite number 0–100 and carries it.
/// - `resets_at` = the time that window resets, epoch seconds.
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
    /// Picked up from a conversation stream.
    Passive,
    /// Received by query.
    Active,
}

/// One usage observation of one vendor.
///
/// ★A missing field (`None`) means 「안 실렸다」 (not carried) regardless of source★ — the receiving side leaves the value it held as is
///   (the same for `model_scoped` · `plan`). A null window in a query result is not 「없음」 (none) either.
/// `model_scoped: Some` replaces the whole list — it is not merged per item. `Some(vec![])` is
///   「모델별 창 없음」 (no per-model window) — the receiver empties the list it holds (the producer emits this when it read the container and it had 0 items).
/// ★Does not carry the observation time★ — the receiving daemon stamps it. The producer (decoder) has no clock.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageObservation {
    pub vendor: UsageVendorKey,
    pub five_hour: Option<WindowObs>,
    pub weekly: Option<WindowObs>,
    pub model_scoped: Option<Vec<ScopedWindowObs>>,
    pub plan: Option<String>,
    pub source: UsageSource,
    /// `Some` = the query succeeded but upstream answered 「이 계정엔 한도 정보가 없다」 (this account has no limit information) — the value is its grounds (Claude
    /// `rate_limits_available: false` — API key · Bedrock · Vertex · a token without profile permission · logged out).
    /// If `Some`, the window fields (`five_hour` · `weekly` · `model_scoped`) are all `None` and `source` is
    /// `Active` — a promise kept by the maker. The receiver empties all window values and folds into the 「정보 없음」 (no information)
    /// state — it is not a failure (a failure leaves the held values as they are).
    /// A `Passive` observation is always `None`.
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
            "기본 구현이 다른 훅으로 새면 안 된다"
        );
    }

    /// The account word goes out over the wire (`account_key`) and to disk — if changed, it no longer matches stored entries.
    #[test]
    fn the_default_account_word_is_default() {
        assert_eq!(UsageAccountKey::default().as_str(), "default");
    }

    /// The daemon's subscription set (`BTreeSet`) iterates in this order — it must be string order, not the word's address, so the same word
    /// becomes one element and comparing sets works.
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
            "다른 주소의 같은 낱말은 같은 키다"
        );
    }
}
