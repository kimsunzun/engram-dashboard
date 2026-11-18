//! Pickup block — lets through no usage observation emitted by the decoder of an agent that is not the default account.
//!
//! ★Why block★: the receiving side's usage entry is the single `(vendor, default account)`; observations of an agent spawned with another
//!   account would mix into it. Which env tells accounts apart is vendor knowledge, decided by each vendor backend; this module has only
//!   the wrapper applying that decision and the env comparison.

use std::ffi::OsStr;

use crate::transport::OutputDecoder;
use crate::types::OutputEvent;

use super::UsageObservation;

/// A decoder wrapper that blocks usage — [`OutputDecoder::take_usage`] is always empty and output events pass through. Only an agent
/// decided not to pick up is wrapped (decision = each vendor backend's `gate_decoder`).
///
/// ★Forward **all** the trait's methods inward — even those with a default body★: an unforwarded method compiles but silently takes the
///   default behavior once wrapped, damaging only non-default-account agents (the only wrapped ones). Today = `decode` · `flush` ·
///   `take_usage`; add forwarding when the trait gains a method.
pub struct UsageGate {
    inner: Box<dyn OutputDecoder>,
}

impl UsageGate {
    pub fn blocking(inner: Box<dyn OutputDecoder>) -> Self {
        Self { inner }
    }
}

impl OutputDecoder for UsageGate {
    fn decode(&mut self, chunk: &[u8]) -> Vec<OutputEvent> {
        self.inner.decode(chunk)
    }

    fn flush(&mut self) -> Vec<OutputEvent> {
        self.inner.flush()
    }

    fn take_usage(&mut self) -> Vec<UsageObservation> {
        drop(self.inner.take_usage());
        Vec::new()
    }
}

/// Does the spawn spec's env set `key` **differently** from the daemon's own value — the neutral part of the pickup decision on the account env.
///
/// - `spec_env` = env **added** for the child (the rest inherited from the daemon); for a repeated key the last wins (spawn applies in order).
/// - `daemon_value` = the daemon process's own value, read and passed by the caller (this function does not read the process env).
/// - Result: `false` if the spec has no such key (inherited) · `false` if present and equal to the daemon value · `true` if absent from
///   the daemon but present in the spec, or the value differs.
///
/// ★Keys compare ASCII case-insensitively and values byte for byte★ — erring toward 「different」 costs only a few of that agent's
///   observations, while the other way mixes another account into the default entry. So no path normalization (trailing separator · case)
///   and no folding an empty value into 「absent」. (Windows env names are case-insensitive; on POSIX this just catches more.)
pub fn env_overrides_daemon(
    spec_env: &[(String, String)],
    key: &str,
    daemon_value: Option<&OsStr>,
) -> bool {
    let Some((_, spec_value)) = spec_env
        .iter()
        .rev()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
    else {
        return false;
    };
    daemon_value != Some(OsStr::new(spec_value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::{UsageSource, UsageVendorKey, WindowObs};
    use std::sync::{Arc, Mutex};

    fn observation(pct: f64) -> UsageObservation {
        UsageObservation {
            vendor: UsageVendorKey::new("test-vendor"),
            five_hour: Some(WindowObs {
                used_pct: Some(pct),
                resets_at: Some(1_900_000_000),
            }),
            weekly: None,
            model_scoped: None,
            plan: None,
            source: UsageSource::Passive,
            limits_unavailable: None,
        }
    }

    /// A decoder that makes its own output per method — if the wrapper fails to forward one inward, that output disappears. The
    /// accumulated observations (`pending`) are visible from outside — that is where we measure whether the wrapper emptied the inside.
    struct Distinct {
        decoded: Arc<Mutex<Vec<u8>>>,
        pending: Arc<Mutex<Vec<UsageObservation>>>,
    }

    impl OutputDecoder for Distinct {
        fn decode(&mut self, chunk: &[u8]) -> Vec<OutputEvent> {
            self.decoded.lock().unwrap().extend_from_slice(chunk);
            self.pending.lock().unwrap().push(observation(40.0));
            vec![OutputEvent::Error("from-decode".into())]
        }
        fn flush(&mut self) -> Vec<OutputEvent> {
            self.pending.lock().unwrap().push(observation(50.0));
            vec![OutputEvent::Error("from-flush".into())]
        }
        fn take_usage(&mut self) -> Vec<UsageObservation> {
            std::mem::take(&mut *self.pending.lock().unwrap())
        }
    }

    type Pending = Arc<Mutex<Vec<UsageObservation>>>;

    fn gate() -> (UsageGate, Arc<Mutex<Vec<u8>>>, Pending) {
        let decoded = Arc::new(Mutex::new(Vec::new()));
        let pending: Pending = Arc::new(Mutex::new(Vec::new()));
        let inner = Distinct {
            decoded: decoded.clone(),
            pending: pending.clone(),
        };
        (UsageGate::blocking(Box::new(inner)), decoded, pending)
    }

    fn error_texts(events: &[OutputEvent]) -> Vec<&str> {
        events
            .iter()
            .map(|e| match e {
                OutputEvent::Error(s) => s.as_str(),
                other => panic!("unexpected event: {other:?}"),
            })
            .collect()
    }

    // ── Forwarding (including `take_usage`, which has a default body) ──

    #[test]
    fn blocking_gate_forwards_every_method_but_never_usage() {
        let (mut g, decoded, pending) = gate();
        assert_eq!(error_texts(&g.decode(b"xyz")), vec!["from-decode"]);
        assert_eq!(decoded.lock().unwrap().as_slice(), b"xyz");
        assert!(g.take_usage().is_empty());
        assert_eq!(error_texts(&g.flush()), vec!["from-flush"]);
        assert!(g.take_usage().is_empty());
        assert!(
            pending.lock().unwrap().is_empty(),
            "if take_usage is not passed through to the inner one, it becomes the trait default (empty vector) and the inner one is not drained"
        );
    }

    /// The blocking wrapper also empties the inside — otherwise the inner vector grows for the agent's whole life.
    #[test]
    fn blocking_gate_still_drains_the_inner_decoder() {
        let (mut g, _, pending) = gate();
        for _ in 0..3 {
            g.decode(b".");
            assert!(g.take_usage().is_empty());
            assert!(
                pending.lock().unwrap().is_empty(),
                "observations discarded while blocking must not remain in the inner one"
            );
        }
    }

    // ── env decision ──

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn missing_key_inherits_the_daemon_value() {
        assert!(!env_overrides_daemon(&env(&[]), "ACCOUNT_DIR", None));
        assert!(!env_overrides_daemon(
            &env(&[("OTHER", "x")]),
            "ACCOUNT_DIR",
            Some(OsStr::new("d"))
        ));
    }

    #[test]
    fn equal_value_is_not_an_override() {
        assert!(!env_overrides_daemon(
            &env(&[("ACCOUNT_DIR", "C:\\a")]),
            "ACCOUNT_DIR",
            Some(OsStr::new("C:\\a"))
        ));
    }

    #[test]
    fn set_against_unset_or_different_is_an_override() {
        assert!(env_overrides_daemon(
            &env(&[("ACCOUNT_DIR", "C:\\a")]),
            "ACCOUNT_DIR",
            None
        ));
        assert!(env_overrides_daemon(
            &env(&[("ACCOUNT_DIR", "C:\\b")]),
            "ACCOUNT_DIR",
            Some(OsStr::new("C:\\a"))
        ));
        assert!(
            env_overrides_daemon(&env(&[("ACCOUNT_DIR", "")]), "ACCOUNT_DIR", None),
            "an empty value also counts as present"
        );
        assert!(
            env_overrides_daemon(
                &env(&[("ACCOUNT_DIR", "C:\\a\\")]),
                "ACCOUNT_DIR",
                Some(OsStr::new("C:\\a"))
            ),
            "does not normalize paths — errs on the side of different"
        );
    }

    #[test]
    fn key_match_ignores_ascii_case_and_last_one_wins() {
        assert!(env_overrides_daemon(
            &env(&[("account_dir", "C:\\b")]),
            "ACCOUNT_DIR",
            Some(OsStr::new("C:\\a"))
        ));
        assert!(!env_overrides_daemon(
            &env(&[("ACCOUNT_DIR", "C:\\b"), ("Account_Dir", "C:\\a")]),
            "ACCOUNT_DIR",
            Some(OsStr::new("C:\\a"))
        ));
        assert!(env_overrides_daemon(
            &env(&[("ACCOUNT_DIR", "C:\\a"), ("ACCOUNT_DIR", "C:\\b")]),
            "ACCOUNT_DIR",
            Some(OsStr::new("C:\\a"))
        ));
    }
}
