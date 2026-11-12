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

/// Test registries. [`fake::WordBackends`] — backends that only know their word, for tests whose subject asks the
/// registry nothing but whether a word is known (every other port method panics). [`fake::Registry`] — agent's real
/// checks plus [`fake::Scripted`] backends. A fake backend stands for a backend agent registers later.
#[cfg(test)]
pub(crate) mod fake {
    use std::path::Path;

    use engram_dashboard_agent::backend::settings_check::{
        CommandLine, FolderEntry, FolderLoadSet, FrontMatterParser, ModeLoads, NativePlaces,
        NativeSkillRoots, NoticeCodes, SkillFormat,
    };
    use engram_dashboard_agent::backend::SettingsCheck;
    use engram_dashboard_agent::launch::Reason;
    use engram_dashboard_agent::profile::AgentOutputFormat;
    use serde_json::Value;

    use super::Backends;

    pub(crate) struct WordOnly(pub &'static str);

    pub(crate) struct WordBackends(pub Vec<WordOnly>);

    impl WordBackends {
        pub(crate) fn new(words: &[&'static str]) -> Self {
            Self(words.iter().map(|&word| WordOnly(word)).collect())
        }
    }

    impl Backends for WordBackends {
        fn for_word(&self, word: &str) -> Option<&dyn SettingsCheck> {
            self.0
                .iter()
                .find(|check| check.0.eq_ignore_ascii_case(word))
                .map(|check| check as &dyn SettingsCheck)
        }

        fn all(&self) -> Vec<&dyn SettingsCheck> {
            self.0
                .iter()
                .map(|check| check as &dyn SettingsCheck)
                .collect()
        }
    }

    impl SettingsCheck for WordOnly {
        fn backend(&self) -> &'static str {
            self.0
        }
        fn loads(&self, _: AgentOutputFormat) -> ModeLoads {
            unimplemented!("word-only fake")
        }
        fn accepts_skill_name(&self, _: &str, _: AgentOutputFormat) -> bool {
            unimplemented!("word-only fake")
        }
        fn folder_entry_blocks(&self, _: &Path) -> bool {
            unimplemented!("word-only fake")
        }
        fn working_folder_blocks(&self, _: &Path, _: AgentOutputFormat) -> Option<Reason> {
            unimplemented!("word-only fake")
        }
        fn notice_codes(&self, _: AgentOutputFormat) -> NoticeCodes {
            unimplemented!("word-only fake")
        }
        fn folder_listing_at_launch(&self, _: AgentOutputFormat) -> bool {
            unimplemented!("word-only fake")
        }
        fn folder_load_set(
            &self,
            _: &[FolderEntry],
            _: &dyn Fn(&Path) -> std::io::Result<String>,
            _: &dyn FrontMatterParser,
            _: AgentOutputFormat,
        ) -> FolderLoadSet {
            unimplemented!("word-only fake")
        }
        fn skill_format(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: &dyn FrontMatterParser,
            _: AgentOutputFormat,
        ) -> SkillFormat {
            unimplemented!("word-only fake")
        }
        fn command_line_args(&self, _: &Value) -> Result<Vec<String>, Reason> {
            unimplemented!("word-only fake")
        }
        fn command_line_fits(&self, _: &CommandLine<'_>) -> Result<(), Reason> {
            unimplemented!("word-only fake")
        }
        fn native_skill_roots(
            &self,
            _: &NativePlaces<'_>,
            _: AgentOutputFormat,
        ) -> NativeSkillRoots {
            unimplemented!("word-only fake")
        }
        fn root_skills(
            &self,
            _: &[FolderEntry],
            _: &dyn Fn(&Path) -> std::io::Result<String>,
            _: &dyn FrontMatterParser,
        ) -> FolderLoadSet {
            unimplemented!("word-only fake")
        }
    }

    /// A registry of any checks — agent's real ones beside [`Scripted`] fakes that stand for a backend agent registers
    /// later. Fakes are leaked so every slot is `'static` like the real registry's.
    pub(crate) struct Registry(pub Vec<&'static dyn SettingsCheck>);

    impl Registry {
        /// agent's real checks, then `extra` in order.
        pub(crate) fn real_and(extra: Vec<Scripted>) -> Self {
            let real: &'static super::AgentBackends = &super::AgentBackends;
            let mut checks: Vec<&'static dyn SettingsCheck> = real.all();
            checks.extend(extra.into_iter().map(|check| {
                let leaked: &'static Scripted = Box::leak(Box::new(check));
                leaked as &'static dyn SettingsCheck
            }));
            Self(checks)
        }
    }

    impl Backends for Registry {
        fn for_word(&self, word: &str) -> Option<&dyn SettingsCheck> {
            self.0
                .iter()
                .find(|check| check.backend().eq_ignore_ascii_case(word))
                .copied()
        }

        fn all(&self) -> Vec<&dyn SettingsCheck> {
            self.0.clone()
        }
    }

    /// A fake backend with simple, fixed answers — what a test needs from a backend no code here knows by name:
    ///
    /// - `command_line_args`: an object holding the key `unsafe` is refused (`<word>-line-unsafe`), carrying the line
    ///   that key's value names when it is a number (a line the port cannot know — the check must replace it), else
    ///   no line; anything else is `Ok([])`.
    /// - `skill_format`: `Invalid(<word>-skill-refused)` when the text holds `refuse`, else `Valid` named after the
    ///   folder.
    ///
    /// Every other port method panics.
    pub(crate) struct Scripted {
        pub word: &'static str,
    }

    impl Scripted {
        pub(crate) fn new(word: &'static str) -> Self {
            Self { word }
        }

        fn reason(&self, what: &str, line: Option<u32>) -> Reason {
            Reason {
                code: format!("{}-{what}", self.word),
                args: serde_json::json!({}),
                line,
            }
        }
    }

    impl SettingsCheck for Scripted {
        fn backend(&self) -> &'static str {
            self.word
        }
        fn loads(&self, _: AgentOutputFormat) -> ModeLoads {
            unimplemented!("scripted fake")
        }
        fn accepts_skill_name(&self, _: &str, _: AgentOutputFormat) -> bool {
            unimplemented!("scripted fake")
        }
        fn folder_entry_blocks(&self, _: &Path) -> bool {
            unimplemented!("scripted fake")
        }
        fn working_folder_blocks(&self, _: &Path, _: AgentOutputFormat) -> Option<Reason> {
            unimplemented!("scripted fake")
        }
        fn notice_codes(&self, _: AgentOutputFormat) -> NoticeCodes {
            unimplemented!("scripted fake")
        }
        fn folder_listing_at_launch(&self, _: AgentOutputFormat) -> bool {
            unimplemented!("scripted fake")
        }
        fn folder_load_set(
            &self,
            _: &[FolderEntry],
            _: &dyn Fn(&Path) -> std::io::Result<String>,
            _: &dyn FrontMatterParser,
            _: AgentOutputFormat,
        ) -> FolderLoadSet {
            unimplemented!("scripted fake")
        }
        fn skill_format(
            &self,
            _: &str,
            folder_name: &str,
            text: &str,
            _: &dyn FrontMatterParser,
            _: AgentOutputFormat,
        ) -> SkillFormat {
            if text.contains("refuse") {
                SkillFormat::Invalid(self.reason("skill-refused", None))
            } else {
                SkillFormat::Valid {
                    name: folder_name.to_owned(),
                    description: String::new(),
                }
            }
        }
        fn command_line_args(&self, raw: &Value) -> Result<Vec<String>, Reason> {
            match raw.get("unsafe") {
                Some(at) => {
                    let line = at.as_u64().and_then(|n| u32::try_from(n).ok());
                    Err(self.reason("line-unsafe", line))
                }
                None => Ok(vec![]),
            }
        }
        fn command_line_fits(&self, _: &CommandLine<'_>) -> Result<(), Reason> {
            unimplemented!("scripted fake")
        }
        fn native_skill_roots(
            &self,
            _: &NativePlaces<'_>,
            _: AgentOutputFormat,
        ) -> NativeSkillRoots {
            unimplemented!("scripted fake")
        }
        fn root_skills(
            &self,
            _: &[FolderEntry],
            _: &dyn Fn(&Path) -> std::io::Result<String>,
            _: &dyn FrontMatterParser,
        ) -> FolderLoadSet {
            unimplemented!("scripted fake")
        }
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
