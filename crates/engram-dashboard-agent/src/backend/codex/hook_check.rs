//! The codex post-open hook check — what `hooks/list {threadId}` says about the hooks Engram wrote into a JSON
//! thread's config, turned into run notices ([`CODEX_HOOKS_UNTRUSTED`]). The transport asks once, after the thread
//! opens and before the link reports ready (`transport.rs` `handshake`); this module only judges the answer.
//!
//! - Every expected entry ([`HookTrust`]) must be listed under its key with `trustStatus: "trusted"` and
//!   `enabled: true` — otherwise `not-listed`, `untrusted` or `disabled`.
//! - A `sessionFlags` hook listed trusted and enabled whose key Engram did not write is `foreign` (a persisted
//!   user-layer entry on a session-flags key, a bypass) — the only run-time sign of a hook riding Engram's trust.
//! - A failed `hooks/list` is `unchecked` — never a handshake failure.
//!
//! ★Pure★ — no I/O and no log; the transport logs counts.
// ADR-0341
// ADR-0420

use serde_json::json;

use super::protocol::HookListed;
use crate::launch::{HookTrust, Notice, CODEX_HOOKS_UNTRUSTED};

/// `HookMetadata.source` of the session-flags layer (the thread config and the app-server line).
const SESSION_FLAGS: &str = "sessionFlags";

/// `HookMetadata.trustStatus` of a hook codex runs.
const TRUSTED: &str = "trusted";

/// The verdict `why`s, in the order the notices are given.
const NOT_LISTED: &str = "not-listed";
const UNTRUSTED: &str = "untrusted";
const DISABLED: &str = "disabled";
const FOREIGN: &str = "foreign";
const UNCHECKED: &str = "unchecked";

/// The run notices for `listed` (every listed hook of the thread) against `expected` — empty when every expected entry
/// is listed trusted and enabled and no foreign session-flags hook is trusted and enabled. One notice per `why`, in
/// the order `not-listed`, `untrusted`, `disabled`, `foreign`; each names every origin (or foreign key) once, in
/// order.
// ADR-0420
pub(super) fn verdicts(expected: &[HookTrust], listed: &[HookListed]) -> Vec<Notice> {
    let mut by_why: [(&str, Vec<String>); 4] = [
        (NOT_LISTED, Vec::new()),
        (UNTRUSTED, Vec::new()),
        (DISABLED, Vec::new()),
        (FOREIGN, Vec::new()),
    ];
    for entry in expected {
        let slot = match listed.iter().find(|hook| hook.key == entry.key) {
            None => 0,
            Some(hook) if hook.trust_status != TRUSTED => 1,
            Some(hook) if !hook.enabled => 2,
            Some(_) => continue,
        };
        push_once(&mut by_why[slot].1, name(entry));
    }
    for hook in listed {
        let ours = expected.iter().any(|entry| entry.key == hook.key);
        if !ours && hook.source == SESSION_FLAGS && hook.trust_status == TRUSTED && hook.enabled {
            push_once(&mut by_why[3].1, hook.key.clone());
        }
    }
    by_why
        .into_iter()
        .filter(|(_, hooks)| !hooks.is_empty())
        .map(|(why, hooks)| notice(json!({ "why": why, "hooks": hooks })))
        .collect()
}

/// The run notice of a failed `hooks/list` — naming every expected origin and the failure; none when nothing was
/// expected (a thread config that only carries other-source state entries).
// ADR-0420
pub(super) fn unchecked(expected: &[HookTrust], error: &str) -> Vec<Notice> {
    let mut hooks = Vec::new();
    for entry in expected {
        push_once(&mut hooks, name(entry));
    }
    if hooks.is_empty() {
        return Vec::new();
    }
    vec![notice(
        json!({ "why": UNCHECKED, "hooks": hooks, "error": error }),
    )]
}

/// What a verdict names for an expected entry — its origin, or its key when the caller gave none.
fn name(entry: &HookTrust) -> String {
    if entry.from.is_empty() {
        entry.key.clone()
    } else {
        entry.from.clone()
    }
}

fn push_once(names: &mut Vec<String>, name: String) {
    if !names.contains(&name) {
        names.push(name);
    }
}

fn notice(args: serde_json::Value) -> Notice {
    Notice {
        code: CODEX_HOOKS_UNTRUSTED.to_owned(),
        args,
        line: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: &str, from: &str) -> HookTrust {
        HookTrust {
            key: key.to_owned(),
            hash: format!("sha256:{key}"),
            from: from.to_owned(),
        }
    }

    fn listed(key: &str, source: &str, enabled: bool, trust: &str) -> HookListed {
        HookListed {
            key: key.to_owned(),
            source: source.to_owned(),
            enabled,
            trust_status: trust.to_owned(),
        }
    }

    /// Every expected key listed trusted and enabled, beside a user-layer hook of any state → no notice.
    #[test]
    fn every_expected_hook_trusted_and_enabled_raises_nothing() {
        let expected = [entry("k0", "team/hooks/a"), entry("k1", "team/codex.yaml")];
        let hooks = [
            listed("k0", SESSION_FLAGS, true, TRUSTED),
            listed("k1", SESSION_FLAGS, true, TRUSTED),
            listed("C:/home/config.toml:stop:0:0", "user", false, TRUSTED),
            listed("C:/home/config.toml:stop:1:0", "user", true, "untrusted"),
        ];
        assert_eq!(verdicts(&expected, &hooks), Vec::new());
    }

    /// ★Each verdict by its `why`★ — not listed (H1: a matcher codex cannot compile), not trusted (a hash drift), turned
    /// off, and a foreign trusted session-flags hook; one notice per `why` in a fixed order, each origin once.
    #[test]
    fn each_failure_is_named_under_its_why_once_per_origin() {
        let expected = [
            entry("k0", "team/hooks/a"),
            entry("k1", "team/hooks/a"),
            entry("k2", "team/hooks/b"),
            entry("k3", "team/codex.yaml"),
            entry("k4", ""),
        ];
        let hooks = [
            listed("k2", SESSION_FLAGS, true, "modified"),
            listed("k3", SESSION_FLAGS, false, TRUSTED),
            listed("k4", SESSION_FLAGS, true, TRUSTED),
            listed("kx", SESSION_FLAGS, true, TRUSTED),
            listed("ky", SESSION_FLAGS, true, "untrusted"),
            listed("kz", "user", true, TRUSTED),
        ];
        assert_eq!(
            verdicts(&expected, &hooks),
            [
                notice(json!({ "why": "not-listed", "hooks": ["team/hooks/a"] })),
                notice(json!({ "why": "untrusted", "hooks": ["team/hooks/b"] })),
                notice(json!({ "why": "disabled", "hooks": ["team/codex.yaml"] })),
                notice(json!({ "why": "foreign", "hooks": ["kx"] })),
            ]
        );
    }

    /// A foreign session-flags hook that is trusted but turned off cannot run — no `foreign` notice.
    #[test]
    fn a_foreign_trusted_but_disabled_session_hook_is_not_named() {
        let hooks = [
            listed("k0", SESSION_FLAGS, true, TRUSTED),
            listed("kx", SESSION_FLAGS, false, TRUSTED),
        ];
        assert_eq!(verdicts(&[entry("k0", "team/hooks/a")], &hooks), Vec::new());
    }

    /// An entry without an origin is named by its key.
    #[test]
    fn an_entry_without_an_origin_is_named_by_its_key() {
        assert_eq!(
            verdicts(&[entry("k0", "")], &[]),
            [notice(json!({ "why": "not-listed", "hooks": ["k0"] }))]
        );
    }

    /// A failed `hooks/list` names every expected origin and the error; with nothing expected it raises nothing.
    #[test]
    fn a_failed_list_is_unchecked_unless_nothing_was_expected() {
        let expected = [entry("k0", "team/hooks/a"), entry("k1", "team/hooks/a")];
        assert_eq!(
            unchecked(&expected, "method not found"),
            [notice(
                json!({ "why": "unchecked", "hooks": ["team/hooks/a"], "error": "method not found" })
            )]
        );
        assert_eq!(unchecked(&[], "method not found"), Vec::new());
    }
}
