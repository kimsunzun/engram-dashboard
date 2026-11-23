//! Shell-made UI text in both UI languages — tray menu labels, the command-refusal box, and the words of automatic tab names. Pure: every function takes the [`Lang`] explicitly; nothing here knows Tauri.
//! There is no process-global language: the layout's language is a `ViewManager` field (changed under the layout lock, bumping the version when an automatic tab name changes), and the tray and the refusal box read the effective `ui.language` setting when needed. A global would make parallel tests share one language and let a late settings hook apply an older value last.
//! The frontend has its own tables (`src/i18n/en.ts` · `ko.ts`) — the shell does not read them; the one word pinned equal on both sides is the `Foreign` tab-name word, by `layout::tab_name`'s test against `slot.foreignContent`.
// ADR-0372 — the shell keeps its own small en/ko table
// ADR-0373 — no process-global language

use crate::daemon_client::refusal::RefusalSite;
use crate::settings::{SettingsService, UI_LANGUAGE};
use crate::tray::core::MenuAction;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    #[default]
    En,
    Ko,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::En, Lang::Ko];

    pub const fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Ko => "ko",
        }
    }

    pub fn from_code(code: &str) -> Option<Lang> {
        Lang::ALL.into_iter().find(|lang| lang.code() == code)
    }

    /// The effective `ui.language` now. The service holds only values from the key's choice list, which equals [`Lang::ALL`] (a test pins that), so the default is reached only if that pin breaks.
    pub fn from_settings(settings: &SettingsService) -> Lang {
        settings
            .effective(UI_LANGUAGE)
            .and_then(|code| Lang::from_code(&code))
            .unwrap_or_default()
    }
}

pub(crate) const fn tray_label(lang: Lang, action: MenuAction) -> &'static str {
    match (lang, action) {
        (Lang::En, MenuAction::StartDaemon) => "Start daemon",
        (Lang::En, MenuAction::StopDaemon) => "Stop daemon",
        (Lang::En, MenuAction::QuitApp) => "Quit Engram",
        (Lang::En, MenuAction::ToggleAutostart) => "Start at sign-in",
        (Lang::Ko, MenuAction::StartDaemon) => "데몬 켜기",
        (Lang::Ko, MenuAction::StopDaemon) => "데몬 끄기",
        (Lang::Ko, MenuAction::QuitApp) => "완전 종료",
        (Lang::Ko, MenuAction::ToggleAutostart) => "부팅 시 자동 시작",
    }
}

pub(crate) const fn refusal_title(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Engram — Command registration refused",
        Lang::Ko => "Engram — 명령 등록 거절",
    }
}

/// The first paragraph of the command-refusal box — what was refused at `site` and what the LLM loses.
pub(crate) const fn refusal_lead(lang: Lang, site: RefusalSite) -> &'static str {
    match (lang, site) {
        (Lang::En, RefusalSite::Register) => {
            "The daemon refused all of this shell's command registrations.\nThe LLM cannot use any window, tab, or slot command until the cause is fixed."
        }
        (Lang::En, RefusalSite::Update) => {
            "The daemon refused to update the list of screen commands.\nThe LLM cannot use the changed screen commands. If the shell reconnects before the cause is fixed, the daemon may refuse all of its command registrations."
        }
        (Lang::Ko, RefusalSite::Register) => {
            "데몬이 이 셸의 명령 등록을 통째로 거절했습니다.\nLLM 이 창 · 탭 · 슬롯 명령을 하나도 쓸 수 없습니다(원인이 고쳐질 때까지)." // hangul-ok: the Korean UI table
        }
        (Lang::Ko, RefusalSite::Update) => {
            "데몬이 화면 명령 명단 갱신을 거절했습니다.\n바뀐 화면 명령을 LLM 이 쓸 수 없고, 원인을 고치지 않은 채 재연결하면 이 셸의 명령 등록 전체가 거절될 수 있습니다." // hangul-ok: the Korean UI table
        }
    }
}

/// The body of the command-refusal box — `lead`, then the daemon's text verbatim, then what to do.
pub(crate) fn refusal_body(lang: Lang, lead: &str, daemon_message: &str) -> String {
    match lang {
        Lang::En => format!(
            "{lead}\n\nDaemon reply:\n{daemon_message}\n\nFix the command declaration that caused it, then rebuild."
        ),
        Lang::Ko => format!(
            "{lead}\n\n데몬 답:\n{daemon_message}\n\n원인이 된 명령 선언을 고친 뒤 다시 빌드하세요." // hangul-ok: the Korean UI table
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabWord {
    EmptySlot,
    AgentUnknown,
    AgentTree,
    Presets,
    Usage,
    /// The PC list slot (ADR-0394).
    Peers,
    /// A slot holding content this build does not know — not a `SlotContent` variant (the in-memory content is `Empty`, the
    /// raw value sits in a side table).
    Foreign,
}

impl TabWord {
    // Add a new variant here too — a word missing from this list passes `tab_name::is_word` and is seeded as an agent name.
    pub const ALL: [TabWord; 7] = [
        TabWord::EmptySlot,
        TabWord::AgentUnknown,
        TabWord::AgentTree,
        TabWord::Presets,
        TabWord::Usage,
        TabWord::Peers,
        TabWord::Foreign,
    ];
}

/// The text of `word` — the `Foreign` word equals `slot.foreignContent` of the frontend table of the same language.
// ADR-0301 — tab names and OS titles (user decision T2 for the Korean words)
pub const fn tab_word(lang: Lang, word: TabWord) -> &'static str {
    match (lang, word) {
        (Lang::En, TabWord::EmptySlot) => "Empty slot",
        (Lang::En, TabWord::AgentUnknown) => "Agent",
        (Lang::En, TabWord::AgentTree) => "Agent tree",
        (Lang::En, TabWord::Presets) => "Presets",
        (Lang::En, TabWord::Usage) => "Usage",
        (Lang::En, TabWord::Peers) => "PCs",
        (Lang::En, TabWord::Foreign) => "Unknown content",
        (Lang::Ko, TabWord::EmptySlot) => "빈 칸",
        (Lang::Ko, TabWord::AgentUnknown) => "에이전트",
        (Lang::Ko, TabWord::AgentTree) => "에이전트 트리",
        (Lang::Ko, TabWord::Presets) => "프리셋",
        (Lang::Ko, TabWord::Usage) => "사용량",
        (Lang::Ko, TabWord::Peers) => "PC 목록",
        (Lang::Ko, TabWord::Foreign) => "알 수 없는 내용",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn assert_distinct_and_filled(texts: &[&str], what: &str) {
        assert!(
            texts.iter().all(|t| !t.trim().is_empty()),
            "{what}: {texts:?}"
        );
        let unique: HashSet<&&str> = texts.iter().collect();
        assert_eq!(unique.len(), texts.len(), "{what}: {texts:?}");
    }

    #[test]
    fn lang_codes_round_trip_and_nothing_else_parses() {
        for lang in Lang::ALL {
            assert_eq!(Lang::from_code(lang.code()), Some(lang));
        }
        for other in ["", "EN", "Ko", "fr", " en", "en-US"] {
            assert_eq!(Lang::from_code(other), None, "{other:?}");
        }
        assert_eq!(Lang::default(), Lang::En);
    }

    #[test]
    fn every_tray_label_exists_in_both_languages() {
        for lang in Lang::ALL {
            let labels: Vec<&str> = MenuAction::ALL
                .iter()
                .map(|&a| tray_label(lang, a))
                .collect();
            assert_distinct_and_filled(&labels, &format!("{lang:?} tray"));
        }
        assert_eq!(tray_label(Lang::En, MenuAction::QuitApp), "Quit Engram");
        assert_eq!(tray_label(Lang::Ko, MenuAction::QuitApp), "완전 종료"); // hangul-ok: the Korean UI table
    }

    #[test]
    fn every_refusal_text_exists_in_both_languages() {
        for lang in Lang::ALL {
            let leads = [
                refusal_lead(lang, RefusalSite::Register),
                refusal_lead(lang, RefusalSite::Update),
            ];
            assert_distinct_and_filled(&leads, &format!("{lang:?} refusal leads"));
            assert!(!refusal_title(lang).trim().is_empty());
            let body = refusal_body(lang, "LEAD", "DAEMON");
            assert!(body.starts_with("LEAD\n\n"), "{lang:?}: {body}");
            assert!(body.contains(":\nDAEMON\n\n"), "{lang:?}: {body}");
        }
        assert_ne!(refusal_title(Lang::En), refusal_title(Lang::Ko));
    }

    #[test]
    fn every_tab_word_exists_in_both_languages_and_the_languages_do_not_share_a_word() {
        let mut all = Vec::new();
        for lang in Lang::ALL {
            let words: Vec<&str> = TabWord::ALL.iter().map(|&w| tab_word(lang, w)).collect();
            assert_distinct_and_filled(&words, &format!("{lang:?} tab words"));
            all.extend(words);
        }
        assert_distinct_and_filled(&all, "both languages");
    }
}
