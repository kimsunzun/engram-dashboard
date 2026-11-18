//! Pure computation of tab and window names (TRD S26-window-chrome §4-2 · §4-4 · PRD R3 · R4) — the effective tab name (`effective_tab_name`), the slot display name (`slot_label`) and the OS window title (`window_title`). It does not know Tauri and only reads the model (`effective_tab_name` reads the `View`, and the name ledger, unknown-content side table and language of the `ViewManager`). The words live in `crate::ui_text` in both UI languages; the language is passed explicitly — the layout's language is the `ViewManager` field, there is no process-global one.
//! D17 — keep `slot_label` a column-0 function whose body holds a single `match` over `SlotContent`: every arm starts with a `SlotContent::<variant>` path pattern (arms joined by `|` too — no wildcard · name binding · `x @ …` · `crate::…` path), guards stay short and never use `||` (a guard rustfmt moves to the next line trips ②), one arm per line, and no other `=>` in the body (a nested `match` etc.). Keep no attribute in this file that makes rustfmt skip anything (② relies on the rustfmt shape — the gate looks for that name in the whole file). The gate (CI `shell gate 2`) reads that body (from the column-0 `pub fn slot_label` line to the first column-0 `}`, outside comments) and checks ① every variant of `types.rs` appears as `SlotContent::<variant>` and ② the arm lines have the shape above — CI does not run clippy, so the lint attribute alone does not hold it. Moving the `match` into another function hides it from the gate.

use super::manager::ViewManager;
use super::tree;
use super::types::{SlotContent, TabName, View};
use crate::ui_text::{tab_word, Lang, TabWord};

const PRODUCT: &str = "Engram";

/// Whether `text` is exactly a shell word of either language. A saved automatic tab name is the [`slot_label`] of that moment, so the tab of an agent missing from the name ledger was saved as the agent word — this keeps such words (of a run in either language, including today's Korean files) out of the ledger seed (TRD S26-window-chrome §4-3). Auto vs named is decided by `TabEntry.named`, not by this.
// ADR-0373
pub(crate) fn is_word(text: &str) -> bool {
    Lang::ALL
        .iter()
        .any(|&lang| TabWord::ALL.iter().any(|&w| tab_word(lang, w) == text))
}

/// The visible name of a tab (R4 · AC4 · AC35) in the manager's language — a named tab's name, or for an automatic tab the [`slot_label`] of its focused slot. A missing focus or one pointing outside the tree falls back to the tree's first slot (same rule as `ViewManager::fixup_focus`). A slot holding content this build does not know is the `Foreign` word — the in-memory content (`Empty`) would call an occupied slot empty. Agent names come from `mgr`'s name ledger. Nothing is stored; computed on every call.
// ADR-0301
pub fn effective_tab_name(mgr: &ViewManager, view: &View) -> String {
    match &view.name {
        TabName::Named(name) => name.clone(),
        TabName::Auto => {
            let lang = mgr.language();
            let slot = view
                .focused_slot_id
                .filter(|&id| tree::contains_slot(&view.layout, id))
                .unwrap_or_else(|| tree::first_slot_id(&view.layout));
            if mgr.unknown_content(view.id, slot).is_some() {
                return tab_word(lang, TabWord::Foreign).to_owned();
            }
            match tree::find_slot(&view.layout, slot) {
                Some(content) => slot_label(lang, content, |id: &str| mgr.agent_name(id)),
                None => tab_word(lang, TabWord::EmptySlot).to_owned(),
            }
        }
    }
}

/// The display name of one slot in `lang` — what an automatic tab name is made of. An agent slot is the name `agent(agent_id)` gives; `None` (not in the ledger) or empty · blank → the agent word (no empty tab name, no 「 - Engram:<n>」 title). A given name is used as is, untrimmed.
// D17: no wildcard or grouped-name arm — adding a slot kind does not compile until it is named.
// ADR-0301
#[deny(clippy::wildcard_enum_match_arm)]
pub fn slot_label<'n>(
    lang: Lang,
    content: &SlotContent,
    agent: impl for<'a> Fn(&'a str) -> Option<&'n str>,
) -> String {
    match content {
        SlotContent::Empty => tab_word(lang, TabWord::EmptySlot).to_owned(),
        SlotContent::Agent { agent_id } => agent(agent_id)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(tab_word(lang, TabWord::AgentUnknown))
            .to_owned(),
        SlotContent::AgentList => tab_word(lang, TabWord::AgentTree).to_owned(),
        SlotContent::PresetPalette => tab_word(lang, TabWord::Presets).to_owned(),
        SlotContent::Usage {
            show_claude: _,
            show_codex: _,
        } => tab_word(lang, TabWord::Usage).to_owned(),
    }
}

/// OS window title (R3) — 「<active tab name> - Engram:<n>」 · 「Engram:<n>」 with no tab yet · for no number (a window outside the model — a hidden prewarmed window) just 「Engram」 regardless of tabs. The name is not truncated — taskbar truncation is the OS's job.
pub fn window_title(active_tab: Option<&str>, number: Option<u32>) -> String {
    match (active_tab, number) {
        (Some(tab), Some(n)) => format!("{tab} - {PRODUCT}:{n}"),
        (None, Some(n)) => format!("{PRODUCT}:{n}"),
        (Some(_), None) | (None, None) => PRODUCT.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::manager::MAIN_WINDOW_LABEL;
    use crate::layout::types::SplitDir;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn no_names(_: &str) -> Option<&'static str> {
        None
    }

    fn agent(id: &str) -> SlotContent {
        SlotContent::Agent {
            agent_id: id.to_owned(),
        }
    }

    fn korean() -> ViewManager {
        let mut mgr = ViewManager::new();
        mgr.set_language(Lang::Ko);
        mgr
    }

    // ── word ──

    #[test]
    fn words_are_the_t2_table() {
        assert_eq!(tab_word(Lang::Ko, TabWord::EmptySlot), "빈 칸"); // hangul-ok: the Korean UI table
        assert_eq!(tab_word(Lang::Ko, TabWord::AgentUnknown), "에이전트"); // hangul-ok: the Korean UI table
        assert_eq!(tab_word(Lang::Ko, TabWord::AgentTree), "에이전트 트리"); // hangul-ok: the Korean UI table
        assert_eq!(tab_word(Lang::Ko, TabWord::Presets), "프리셋"); // hangul-ok: the Korean UI table
        assert_eq!(tab_word(Lang::Ko, TabWord::Usage), "사용량"); // hangul-ok: the Korean UI table
        assert_eq!(tab_word(Lang::Ko, TabWord::Foreign), "알 수 없는 내용"); // hangul-ok: the Korean UI table
    }

    #[test]
    fn english_words() {
        assert_eq!(tab_word(Lang::En, TabWord::EmptySlot), "Empty slot");
        assert_eq!(tab_word(Lang::En, TabWord::AgentUnknown), "Agent");
        assert_eq!(tab_word(Lang::En, TabWord::AgentTree), "Agent tree");
        assert_eq!(tab_word(Lang::En, TabWord::Presets), "Presets");
        assert_eq!(tab_word(Lang::En, TabWord::Usage), "Usage");
        assert_eq!(tab_word(Lang::En, TabWord::Foreign), "Unknown content");
    }

    // Reads the frontend tables as source text — so it pins their paths and the one-line form `foreignContent: '…'`.
    #[test]
    fn foreign_word_is_the_one_the_frontend_shows() {
        for (lang, table, path) in [
            (
                Lang::En,
                include_str!("../../../src/i18n/en.ts"),
                "src/i18n/en.ts",
            ),
            (
                Lang::Ko,
                include_str!("../../../src/i18n/ko.ts"),
                "src/i18n/ko.ts",
            ),
        ] {
            assert!(
                table.contains(&format!(
                    "foreignContent: '{}'",
                    tab_word(lang, TabWord::Foreign)
                )),
                "slot.foreignContent of {path}"
            );
        }
    }

    #[test]
    fn every_table_word_is_a_word_and_nothing_else_is() {
        for text in [
            "빈 칸",           // hangul-ok: the Korean UI table
            "에이전트",        // hangul-ok: the Korean UI table
            "에이전트 트리",   // hangul-ok: the Korean UI table
            "프리셋",          // hangul-ok: the Korean UI table
            "사용량",          // hangul-ok: the Korean UI table
            "알 수 없는 내용", // hangul-ok: the Korean UI table
            "Empty slot",
            "Agent",
            "Agent tree",
            "Presets",
            "Usage",
            "Unknown content",
        ] {
            assert!(is_word(text), "{text}");
        }
        for text in [
            "myproj",
            "에이전트 ", // hangul-ok: Korean-word fixture
            " 에이전트", // hangul-ok: Korean-word fixture
            "에이전트1", // hangul-ok: Korean-word fixture
            "View 3",
            "",
            "agent",
            "Agent ",
            "AGENT",
        ] {
            assert!(!is_word(text), "{text:?}");
        }
    }

    // ── slot_label ──

    #[test]
    fn non_agent_slots_use_their_word() {
        assert_eq!(slot_label(Lang::Ko, &SlotContent::Empty, no_names), "빈 칸"); // hangul-ok: the Korean UI table
        assert_eq!(
            slot_label(Lang::Ko, &SlotContent::AgentList, no_names),
            "에이전트 트리" // hangul-ok: the Korean UI table
        );
        assert_eq!(
            slot_label(Lang::Ko, &SlotContent::PresetPalette, no_names),
            "프리셋" // hangul-ok: the Korean UI table
        );
        assert_eq!(
            slot_label(Lang::En, &SlotContent::Empty, no_names),
            "Empty slot"
        );
        assert_eq!(
            slot_label(Lang::En, &SlotContent::AgentList, no_names),
            "Agent tree"
        );
        assert_eq!(
            slot_label(Lang::En, &SlotContent::PresetPalette, no_names),
            "Presets"
        );
    }

    #[test]
    fn usage_slot_is_one_word_whatever_providers_it_shows() {
        for (show_claude, show_codex) in
            [(true, true), (true, false), (false, true), (false, false)]
        {
            let usage = SlotContent::Usage {
                show_claude,
                show_codex,
            };
            assert_eq!(slot_label(Lang::Ko, &usage, no_names), "사용량"); // hangul-ok: the Korean UI table
            assert_eq!(slot_label(Lang::En, &usage, no_names), "Usage");
        }
    }

    #[test]
    fn agent_slot_uses_the_ledger_name() {
        let names = HashMap::from([("a1".to_owned(), "myproj".to_owned())]);
        let lookup = |id: &str| names.get(id).map(String::as_str);
        for lang in Lang::ALL {
            assert_eq!(slot_label(lang, &agent("a1"), lookup), "myproj");
        }
    }

    #[test]
    fn agent_slot_missing_from_ledger_is_agent_word() {
        let names = HashMap::from([("a1".to_owned(), "myproj".to_owned())]);
        let lookup = |id: &str| names.get(id).map(String::as_str);
        assert_eq!(slot_label(Lang::Ko, &agent("a2"), lookup), "에이전트"); // hangul-ok: the Korean UI table
        assert_eq!(slot_label(Lang::Ko, &agent("a1"), no_names), "에이전트"); // hangul-ok: the Korean UI table
        assert_eq!(slot_label(Lang::En, &agent("a2"), lookup), "Agent");
    }

    #[test]
    fn agent_slot_with_blank_ledger_name_is_agent_word() {
        for blank in ["", " ", "   ", "\t\n", "\u{3000}"] {
            assert_eq!(
                slot_label(Lang::Ko, &agent("a1"), |_: &str| Some(blank)),
                "에이전트", // hangul-ok: the Korean UI table
                "{blank:?}"
            );
        }
    }

    #[test]
    fn agent_slot_name_with_surrounding_spaces_is_kept_as_given() {
        assert_eq!(
            slot_label(Lang::En, &agent("a1"), |_: &str| Some(" a b ")),
            " a b "
        );
    }

    #[test]
    fn agent_lookup_receives_the_slot_agent_id() {
        let seen = std::cell::RefCell::new(Vec::new());
        let label = slot_label(Lang::En, &agent("id-42"), |id: &str| {
            seen.borrow_mut().push(id.to_owned());
            Some("n")
        });
        assert_eq!(label, "n");
        assert_eq!(seen.into_inner(), vec!["id-42".to_owned()]);
    }

    #[test]
    fn non_agent_slots_do_not_consult_the_ledger() {
        let called = std::cell::Cell::new(false);
        let lookup = |_: &str| {
            called.set(true);
            Some("x")
        };
        for content in [
            SlotContent::Empty,
            SlotContent::AgentList,
            SlotContent::PresetPalette,
            SlotContent::Usage {
                show_claude: true,
                show_codex: true,
            },
        ] {
            slot_label(Lang::En, &content, lookup);
        }
        assert!(!called.get());
    }

    // ── effective_tab_name ──

    fn main_tab(mgr: &ViewManager) -> Uuid {
        mgr.windows[MAIN_WINDOW_LABEL].active
    }

    fn name_of(mgr: &ViewManager, view: Uuid) -> String {
        effective_tab_name(mgr, &mgr.views[&view])
    }

    // left = agent a1 · right = agent tree. Focus is on the new area (right) (`split_slot`).
    fn agent_and_tree() -> (ViewManager, Uuid, Uuid, Uuid) {
        let mut mgr = korean();
        let view = main_tab(&mgr);
        let left = tree::first_slot_id(&mgr.views[&view].layout);
        let right = mgr.split_slot(view, left, SplitDir::LeftRight).unwrap();
        mgr.assign_agent(view, left, "a1".into()).unwrap();
        mgr.set_slot_content(view, right, SlotContent::AgentList)
            .unwrap();
        mgr.put_agent_name("a1", "myproj");
        (mgr, view, left, right)
    }

    #[test]
    fn an_auto_tab_is_named_after_its_focused_slot() {
        let (mut mgr, view, left, right) = agent_and_tree();
        assert_eq!(name_of(&mgr, view), "에이전트 트리"); // hangul-ok: the Korean UI table
        mgr.set_focused_slot(view, left).unwrap();
        assert_eq!(name_of(&mgr, view), "myproj");
        mgr.set_focused_slot(view, right).unwrap();
        assert_eq!(name_of(&mgr, view), "에이전트 트리"); // hangul-ok: the Korean UI table
    }

    #[test]
    fn a_named_tab_keeps_its_name_whatever_the_focus() {
        let (mut mgr, view, left, _) = agent_and_tree();
        mgr.rename_tab(view, "작업".into()).unwrap(); // hangul-ok: Korean tab-name fixture
        assert_eq!(name_of(&mgr, view), "작업"); // hangul-ok: Korean tab-name fixture
        mgr.set_focused_slot(view, left).unwrap();
        assert_eq!(name_of(&mgr, view), "작업"); // hangul-ok: Korean tab-name fixture
    }

    #[test]
    fn a_focus_that_is_missing_or_outside_the_tree_falls_back_to_the_first_slot() {
        let (mgr, view, _, _) = agent_and_tree();
        let mut detached = mgr.views[&view].clone();
        detached.focused_slot_id = Some(Uuid::new_v4());
        assert_eq!(effective_tab_name(&mgr, &detached), "myproj");
        detached.focused_slot_id = None;
        assert_eq!(effective_tab_name(&mgr, &detached), "myproj");
    }

    #[test]
    fn an_agent_missing_from_the_ledger_is_the_agent_word() {
        let mut mgr = korean();
        let view = main_tab(&mgr);
        let slot = tree::first_slot_id(&mgr.views[&view].layout);
        mgr.assign_agent(view, slot, "a-unknown".into()).unwrap();
        assert_eq!(name_of(&mgr, view), "에이전트"); // hangul-ok: the Korean UI table
    }

    #[test]
    fn auto_names_follow_the_managers_language() {
        let mut mgr = ViewManager::new();
        assert_eq!(mgr.language(), Lang::En, "English by default");
        let view = main_tab(&mgr);
        assert_eq!(name_of(&mgr, view), "Empty slot");
        mgr.set_language(Lang::Ko);
        assert_eq!(name_of(&mgr, view), "빈 칸"); // hangul-ok: the Korean UI table
        let slot = tree::first_slot_id(&mgr.views[&view].layout);
        let raw = serde_json::json!({ "type": "from_the_future" });
        mgr.set_unknown_content(view, slot, raw.as_object().unwrap().clone())
            .unwrap();
        mgr.set_language(Lang::En);
        assert_eq!(name_of(&mgr, view), "Unknown content");
    }

    #[test]
    fn a_slot_holding_unknown_content_is_the_foreign_word_not_empty() {
        let mut mgr = korean();
        let view = main_tab(&mgr);
        let slot = tree::first_slot_id(&mgr.views[&view].layout);
        let raw = serde_json::json!({ "type": "from_the_future" });
        mgr.set_unknown_content(view, slot, raw.as_object().unwrap().clone())
            .unwrap();
        assert_eq!(name_of(&mgr, view), "알 수 없는 내용"); // hangul-ok: the Korean UI table
        mgr.set_slot_content(view, slot, SlotContent::Empty)
            .unwrap();
        assert_eq!(name_of(&mgr, view), "빈 칸"); // hangul-ok: the Korean UI table
    }

    // The only empty area of a new tab · new window is the focus area from the start — what is put there becomes the tab name (§4-2).
    #[test]
    fn placing_content_in_a_new_tabs_only_slot_names_the_tab() {
        let mut mgr = korean();
        let view = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        assert_eq!(name_of(&mgr, view), "빈 칸"); // hangul-ok: the Korean UI table
        let slot = tree::first_slot_id(&mgr.views[&view].layout);
        mgr.set_slot_content(view, slot, SlotContent::PresetPalette)
            .unwrap();
        assert_eq!(name_of(&mgr, view), "프리셋"); // hangul-ok: the Korean UI table
    }

    // Default names (「View N」 · 「Popup N」 · 「Tab」) are not made — every newly standing tab is auto-named.
    #[test]
    fn new_tabs_are_auto_named_everywhere() {
        let mut mgr = korean();
        let boot = main_tab(&mgr);
        let tab = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let popout = mgr.create_window("slot-popup-1").unwrap();
        let slot = tree::first_slot_id(&mgr.views[&boot].layout);
        mgr.set_slot_content(boot, slot, SlotContent::AgentList)
            .unwrap();
        let (detached, _) = mgr.prepare_detached_view(boot, slot).unwrap();
        for view in [boot, tab, popout, detached] {
            assert_eq!(mgr.views[&view].name, TabName::Auto, "{view}");
        }
        assert_eq!(
            name_of(&mgr, detached),
            "에이전트 트리", // hangul-ok: the Korean UI table
            "name of the moved area"
        );
        let listed = mgr.list_tabs(MAIN_WINDOW_LABEL).unwrap();
        assert!(listed.tabs.iter().all(|t| !t.named));
    }

    // ── window_title ──

    #[test]
    fn title_with_tab_and_number() {
        assert_eq!(window_title(Some("myproj"), Some(1)), "myproj - Engram:1");
        assert_eq!(window_title(Some("빈 칸"), Some(12)), "빈 칸 - Engram:12"); // hangul-ok: the Korean UI table
    }

    #[test]
    fn title_without_tab_is_product_and_number() {
        assert_eq!(window_title(None, Some(3)), "Engram:3");
    }

    #[test]
    fn title_without_number_is_product_only() {
        assert_eq!(window_title(None, None), "Engram");
        assert_eq!(window_title(Some("myproj"), None), "Engram");
    }

    #[test]
    fn title_keeps_long_and_unicode_names_whole() {
        let long = "가".repeat(100) + &"a".repeat(100); // hangul-ok: Korean multi-byte fixture
        assert_eq!(long.chars().count(), 200);
        assert_eq!(
            window_title(Some(&long), Some(100)),
            format!("{long} - Engram:100")
        );
        let mixed = "프로젝트 🚀 alpha – β"; // hangul-ok: Korean fixture
        assert_eq!(
            window_title(Some(mixed), Some(2)),
            "프로젝트 🚀 alpha – β - Engram:2" // hangul-ok: Korean fixture
        );
    }
}
