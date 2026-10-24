//! 탭 · 창 이름의 순수 셈(TRD S26-window-chrome §4-2 · §4-4 · PRD R3 · R4) — 슬롯 표시 이름(`slot_label`) ·
//! OS 창 제목(`window_title`) · 셸 낱말 표(`word`). Tauri 도 모델(`View` · `ViewManager`)도 모른다.
//!
//! ★D17 — `slot_label` 은 0 열 함수로 두고 본문에 `SlotContent` 위 `match` 하나만 담는다★: 갈래는 모두
//! `SlotContent::<변형>` 경로 패턴으로 시작하고(`|` 로 이은 갈래도 — 와일드카드 · 이름 묶기 · `x @ …` · `crate::…`
//! 경로 없음), 가드는 짧게 두고 `||` 를 쓰지 않으며(rustfmt 가 다음 줄로 내린 가드는 ② 에 걸린다), 한 줄에 갈래를
//! 하나만 두고, 본문에 다른 `=>`(중첩 `match` 등)를 두지 않는다. 이 파일에는 rustfmt 를 건너뛰게 하는 attribute 를
//! 두지 않는다(② 가 rustfmt 꼴에 기댄다 — 게이트가 파일 전체에서 그 이름을 찾는다). 게이트(CI `shell gate 2` ·
//! `/qa` 바인딩 4j)가 그 본문(0 열 `pub fn slot_label` 줄부터 첫 0 열 `}` 까지 · 주석 밖)을 보고 ① `types.rs` 의 모든
//! 변형이 `SlotContent::<변형>` 으로 나오는지 ② 갈래 줄이 위 꼴인지 잰다 — CI 가 clippy 를 돌리지 않아 lint
//! attribute 만으로는 지켜지지 않는다. `match` 를 다른 함수로 떼어 내면 게이트가 못 본다.

use super::types::SlotContent;

const PRODUCT: &str = "Engram";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Word {
    EmptySlot,
    AgentUnknown,
    AgentTree,
    Presets,
    Usage,
    /// 이 판이 모르는 내용의 슬롯 — `SlotContent` 의 갈래가 아니다(메모리 내용은 `Empty` · 원문은 곁표).
    // TODO(T-66 U3a): effective_tab_name 이 이 변형을 쓰면 아래 allow 를 걷는다(그 전엔 운영 빌드에서 안 만들어진다).
    #[cfg_attr(not(test), allow(dead_code))]
    Foreign,
}

/// 사람에게 보이는 낱말의 셸 정본(사용자 결정 T2 · 2026-10-24) — 셸에는 i18n 층이 없어 여기 한 곳에 둔다.
/// 웹 i18n 과 합치는 것은 T-68 몫이다. `Foreign` 의 글자는 웹 `ko.ts` 의 `slot.foreignContent` 와 같아야 한다.
const fn word(w: Word) -> &'static str {
    match w {
        Word::EmptySlot => "빈 칸",
        Word::AgentUnknown => "에이전트",
        Word::AgentTree => "에이전트 트리",
        Word::Presets => "프리셋",
        Word::Usage => "사용량",
        Word::Foreign => "알 수 없는 내용",
    }
}

/// 슬롯 하나의 표시 이름 — 탭 자동 이름의 재료. 에이전트 슬롯은 `agent(agent_id)` 가 준 이름이고, 그것이
/// `None`(장부에 없음)이거나 빈 글자 · 공백뿐이면 「에이전트」다(빈 탭 이름 · 「 - Engram:<n>」 제목을 막는다).
/// 준 이름은 다듬지 않고 그대로 쓴다.
// D17: 와일드카드 · 묶음 이름 갈래를 두지 않는다 — 슬롯 종류를 더하면 이름을 짓기 전에는 컴파일이 안 된다.
#[deny(clippy::wildcard_enum_match_arm)]
pub fn slot_label<'n>(
    content: &SlotContent,
    agent: impl for<'a> Fn(&'a str) -> Option<&'n str>,
) -> String {
    match content {
        SlotContent::Empty => word(Word::EmptySlot).to_owned(),
        SlotContent::Agent { agent_id } => agent(agent_id)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(word(Word::AgentUnknown))
            .to_owned(),
        SlotContent::AgentList => word(Word::AgentTree).to_owned(),
        SlotContent::PresetPalette => word(Word::Presets).to_owned(),
        SlotContent::Usage {
            show_claude: _,
            show_codex: _,
        } => word(Word::Usage).to_owned(),
    }
}

/// OS 창 제목(R3) — 「<활성 탭 이름> - Engram:<n>」 · 탭이 아직 없으면 「Engram:<n>」 · 번호가 없으면(모델 밖
/// 창 — 숨은 예열 창) 탭과 상관없이 「Engram」. 이름을 자르지 않는다 — 작업 표시줄이 자르는 것은 OS 몫이다.
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
    use std::collections::HashMap;

    fn no_names(_: &str) -> Option<&'static str> {
        None
    }

    fn agent(id: &str) -> SlotContent {
        SlotContent::Agent {
            agent_id: id.to_owned(),
        }
    }

    // ── word ──

    #[test]
    fn words_are_the_t2_table() {
        assert_eq!(word(Word::EmptySlot), "빈 칸");
        assert_eq!(word(Word::AgentUnknown), "에이전트");
        assert_eq!(word(Word::AgentTree), "에이전트 트리");
        assert_eq!(word(Word::Presets), "프리셋");
        assert_eq!(word(Word::Usage), "사용량");
        assert_eq!(word(Word::Foreign), "알 수 없는 내용");
    }

    #[test]
    fn foreign_word_is_the_one_the_frontend_shows() {
        let ko = include_str!("../../../src/i18n/ko.ts");
        assert!(
            ko.contains(&format!("foreignContent: '{}'", word(Word::Foreign))),
            "src/i18n/ko.ts 의 slot.foreignContent"
        );
    }

    // ── slot_label ──

    #[test]
    fn non_agent_slots_use_their_word() {
        assert_eq!(slot_label(&SlotContent::Empty, no_names), "빈 칸");
        assert_eq!(
            slot_label(&SlotContent::AgentList, no_names),
            "에이전트 트리"
        );
        assert_eq!(slot_label(&SlotContent::PresetPalette, no_names), "프리셋");
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
            assert_eq!(slot_label(&usage, no_names), "사용량");
        }
    }

    #[test]
    fn agent_slot_uses_the_ledger_name() {
        let names = HashMap::from([("a1".to_owned(), "myproj".to_owned())]);
        let lookup = |id: &str| names.get(id).map(String::as_str);
        assert_eq!(slot_label(&agent("a1"), lookup), "myproj");
    }

    #[test]
    fn agent_slot_missing_from_ledger_is_agent_word() {
        let names = HashMap::from([("a1".to_owned(), "myproj".to_owned())]);
        let lookup = |id: &str| names.get(id).map(String::as_str);
        assert_eq!(slot_label(&agent("a2"), lookup), "에이전트");
        assert_eq!(slot_label(&agent("a1"), no_names), "에이전트");
    }

    #[test]
    fn agent_slot_with_blank_ledger_name_is_agent_word() {
        for blank in ["", " ", "   ", "\t\n", "\u{3000}"] {
            assert_eq!(
                slot_label(&agent("a1"), |_: &str| Some(blank)),
                "에이전트",
                "{blank:?}"
            );
        }
    }

    #[test]
    fn agent_slot_name_with_surrounding_spaces_is_kept_as_given() {
        assert_eq!(slot_label(&agent("a1"), |_: &str| Some(" a b ")), " a b ");
    }

    #[test]
    fn agent_lookup_receives_the_slot_agent_id() {
        let seen = std::cell::RefCell::new(Vec::new());
        let label = slot_label(&agent("id-42"), |id: &str| {
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
            slot_label(&content, lookup);
        }
        assert!(!called.get());
    }

    // ── window_title ──

    #[test]
    fn title_with_tab_and_number() {
        assert_eq!(window_title(Some("myproj"), Some(1)), "myproj - Engram:1");
        assert_eq!(window_title(Some("빈 칸"), Some(12)), "빈 칸 - Engram:12");
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
        let long = "가".repeat(100) + &"a".repeat(100);
        assert_eq!(long.chars().count(), 200);
        assert_eq!(
            window_title(Some(&long), Some(100)),
            format!("{long} - Engram:100")
        );
        let mixed = "프로젝트 🚀 alpha – β";
        assert_eq!(
            window_title(Some(mixed), Some(2)),
            "프로젝트 🚀 alpha – β - Engram:2"
        );
    }
}
