//! The backend registry seam. Every per-backend answer this crate uses comes from a [`SettingsCheck`] reached through
//! [`Backends`], so a backend added to agent's registry works here with no change, and tests inject a registry with a
//! fake extra backend.
// TRD S25 step-7 §1-1 · §3-3a
// ADR-0334

use engram_dashboard_agent::backend::{settings_check_for, settings_checks, SettingsCheck};

/// The registered backend settings checks.
pub trait Backends: Send + Sync {
    /// The settings check of the backend named `word`, ASCII case-insensitive. `None` = no backend of that word has one
    /// (an unknown tag or raw-field word).
    fn for_word(&self, word: &str) -> Option<&dyn SettingsCheck>;

    /// Every registered settings check, in registry order.
    fn all(&self) -> Vec<&dyn SettingsCheck>;
}

/// The production registry: agent's `settings_check_for` / `settings_checks()`. ★The only caller of those two in this
/// crate★ — everything else takes `&dyn Backends`.
#[derive(Clone, Copy, Debug, Default)]
pub struct AgentBackends;

impl Backends for AgentBackends {
    fn for_word(&self, word: &str) -> Option<&dyn SettingsCheck> {
        settings_check_for(word)
    }

    fn all(&self) -> Vec<&dyn SettingsCheck> {
        settings_checks().into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn same(a: &dyn SettingsCheck, b: &dyn SettingsCheck) -> bool {
        std::ptr::addr_eq(a as *const dyn SettingsCheck, b as *const dyn SettingsCheck)
    }

    #[test]
    fn all_is_the_agent_registry_in_its_order() {
        let ours = AgentBackends.all();
        let agent = settings_checks();
        // The words too: zero-sized statics may share an address, so `same` alone cannot tell two checks apart.
        let words = |checks: &[&dyn SettingsCheck]| -> Vec<&'static str> {
            checks.iter().map(|check| check.backend()).collect()
        };
        assert_eq!(words(&ours), words(&agent));
        assert_eq!(ours.len(), agent.len());
        for (a, b) in ours.iter().zip(agent) {
            assert!(same(*a, b), "{} · {}", a.backend(), b.backend());
        }
    }

    #[test]
    fn for_word_answers_like_the_agent_registry_in_any_case() {
        for check in settings_checks() {
            let word = check.backend();
            for spelled in [
                word.to_owned(),
                word.to_ascii_uppercase(),
                capitalized(word),
            ] {
                let ours = AgentBackends.for_word(&spelled).expect(&spelled);
                assert_eq!(ours.backend(), word, "{spelled}");
                assert!(same(ours, check), "{spelled}");
                let agent = settings_check_for(&spelled).expect(&spelled);
                assert!(same(ours, agent), "{spelled}");
            }
        }
    }

    #[test]
    fn an_unknown_word_has_no_check() {
        for word in ["", "nope", " ", "engram"] {
            assert!(AgentBackends.for_word(word).is_none(), "{word:?}");
            assert!(settings_check_for(word).is_none(), "{word:?}");
        }
    }

    fn capitalized(word: &str) -> String {
        let mut chars = word.chars();
        chars
            .next()
            .map(|first| first.to_ascii_uppercase().to_string() + chars.as_str())
            .unwrap_or_default()
    }
}
