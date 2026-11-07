//! tray core — the **pure logic** of tray behavior (no dependence on OS/GUI/network).
//!
//! Purity invariant: zero tauri/discovery imports — it deals only with slices/enums (CLAUDE.md 「코어 격리」).
//! Seams of the Launcher/DaemonProbe/dispatch kind are **deliberately absent** — in the integrated app the tray handler calls the
//! discovery command directly, so they are unnecessary (ADR-0026, TRD §2).
//! There is no QuitTray in the menu because tray = app integration makes it meaningless (ADR-0026).
//! ★Note: the show_main_ui/hide_main_ui actions·commands are kept★ — for ShowUi/HideUi only the menu *item* was removed
//! (the tray left click replaces it), and LLM/cdp control (CLAUDE.md 「LLM-우선 제어」) keeps using the same actions functions.

use crate::layout::MAIN_WINDOW_LABEL;

// ── Menu intent ──────────────────────────────────────────────────────────────────

// The **intent** a tray menu click expresses. Human clicks · LLM calls · shortcuts all converge
// on this intent (CLAUDE.md 「LLM-우선 제어」).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    StartDaemon,
    StopDaemon,
    QuitApp,
    ToggleAutostart,
}

impl MenuAction {
    // Stable id — unchanged even if the label changes (click mapping stays stable).
    pub const fn menu_id(self) -> &'static str {
        match self {
            MenuAction::StartDaemon => "start_daemon",
            MenuAction::StopDaemon => "stop_daemon",
            MenuAction::QuitApp => "quit_app",
            MenuAction::ToggleAutostart => "toggle_autostart",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            MenuAction::StartDaemon => "데몬 켜기",
            MenuAction::StopDaemon => "데몬 끄기",
            MenuAction::QuitApp => "완전 종료",
            MenuAction::ToggleAutostart => "부팅 시 자동 시작",
        }
    }

    pub const ALL: [MenuAction; 4] = [
        MenuAction::StartDaemon,
        MenuAction::StopDaemon,
        MenuAction::ToggleAutostart,
        MenuAction::QuitApp,
    ];
}

pub fn action_for_menu_id(id: &str) -> Option<MenuAction> {
    MenuAction::ALL.into_iter().find(|a| a.menu_id() == id)
}

// ── State → display mapping (pure) ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconState {
    Active,
    Inactive,
}

pub fn icon_state_for(alive: bool) -> IconState {
    if alive {
        IconState::Active
    } else {
        IconState::Inactive
    }
}

// ── Target windows for show·hide (pure) ──────────────────────────────────────────────────────

// The windows tray show·hide handle = all popouts (ascending number) and then main. Other windows are left out — so the tray does not bring up windows
// that were not made to be shown to a person.
// ★Main being last is the contract★: the caller shows · unminimizes windows and gives focus in the order received, and all three of those activate the window
//   (tao 0.35 Windows = `SW_SHOW` · `SW_RESTORE` · `SetForegroundWindow`) — if a popout is handled after main, it takes
//   the focus. For the same reason the popout with the largest number comes right below main.
// ★Do not keep popouts in the order received★: the `webview_windows()` the caller passes is a new HashMap with a random hasher on every call
//   (tauri 2.11.3 `src/lib.rs:588-601`), so following that order would shuffle the popout stacking order on every show.
// The popout decision is taken as an argument — so that code outside tests does not lean on the Tauri-bound `commands` module,
//   and tests pass the canonical decision (`commands::popout::is_popup_label`) as is to pin down the contract.
// ADR-0229
pub fn ui_windows<'a>(
    labels: impl IntoIterator<Item = &'a str>,
    is_popout: impl Fn(&str) -> bool,
) -> Vec<&'a str> {
    let mut main = None;
    let mut windows = Vec::new();
    for label in labels {
        if label == MAIN_WINDOW_LABEL {
            main = Some(label);
        } else if is_popout(label) {
            windows.push(label);
        }
    }
    // Numbered ones first (by number), unnumbered ones after by label. The same number ("7"·"007") is also split by label to make a total order
    //   — a comparator mixing numeric·text comparison is not a total order and the sort may panic.
    windows.sort_by_key(|&label| {
        let n = popout_number(label);
        (n.is_none(), n, label)
    });
    windows.extend(main);
    windows
}

// Number = the tail after the last `-`. For an issued label (`slot-popup-<실행 표식>-<n>` — TRD S26-window-chrome §4-5) it is the issue sequence number n —
// the prefix constant is in the Tauri-bound `commands::popout` and is not used here. An empty tail · non-digits · over u64 is `None`.
fn popout_number(label: &str) -> Option<u64> {
    label.rsplit_once('-')?.1.parse().ok()
}

// ── Icon pixel conversion (pure) ──────────────────────────────────────────────────────────

// `rgba.len()` must be `w*h*4` (RGBA 4 channels). This precondition is guaranteed by the caller (the `.rgba()` decoded with Tauri `Image::from_bytes`) —
// the decode result is a buffer of a length consistent with `(width, height)`.
// len ≠ w*h*4 is always a contract violation (regardless of being a multiple of 4). A debug build catches it at once with the debug_assert below as a
// panic, and release is caught downstream but **soft** — the direct consumer `Image::new_owned(_, w, h)`
// is an unvalidated constructor, but before that `Image` reaches the tray it goes through the length check of `TryFrom → tray_icon::Icon::from_rgba`
// (builder path = swallowed with `.ok()` so the icon is not set / `set_icon` path = warn
// log degrade — neither is a panic). So debug_assert is not the only line of defense but the only one that catches immediately
// and loudly. If the precondition holds, there is no chunks_exact remainder.
pub fn to_grayscale_rgba(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    debug_assert_eq!(
        rgba.len(),
        (w as usize) * (h as usize) * 4,
        "to_grayscale_rgba: 버퍼 길이 ≠ w*h*4 (RGBA)"
    );
    let mut out = Vec::with_capacity(rgba.len());
    for px in rgba.chunks_exact(4) {
        // Rec.601 luma. Round after f32 accumulation — avoids the accumulated error of integer approximation.
        let luma = 0.299 * px[0] as f32 + 0.587 * px[1] as f32 + 0.114 * px[2] as f32;
        let g = luma.round().clamp(0.0, 255.0) as u8;
        out.push(g);
        out.push(g);
        out.push(g);
        out.push(px[3]);
    }
    out
}

// ── Tests (pure units with no OS/GUI dependence) ─────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_id_roundtrips_to_action() {
        for action in MenuAction::ALL {
            assert_eq!(action_for_menu_id(action.menu_id()), Some(action));
        }
    }

    #[test]
    fn toggle_autostart_id_label_roundtrip() {
        assert_eq!(MenuAction::ToggleAutostart.menu_id(), "toggle_autostart");
        assert_eq!(MenuAction::ToggleAutostart.label(), "부팅 시 자동 시작");
        assert_eq!(
            action_for_menu_id("toggle_autostart"),
            Some(MenuAction::ToggleAutostart)
        );
    }

    #[test]
    fn unknown_menu_id_is_none() {
        assert_eq!(action_for_menu_id("nope"), None);
        assert_eq!(action_for_menu_id(""), None);
    }

    #[test]
    fn icon_state_maps_alive() {
        assert_eq!(icon_state_for(true), IconState::Active);
        assert_eq!(icon_state_for(false), IconState::Inactive);
    }

    #[test]
    fn all_variants_present_in_all_array() {
        // Preventing an ALL omission: if a new variant is added, the exhaustive match below gives a compile error
        // (non-exhaustive) that forces awareness of "did you put this variant in ALL".
        fn assert_in_all(a: MenuAction) {
            assert!(
                MenuAction::ALL.contains(&a),
                "{a:?} 가 MenuAction::ALL 에 없음 — 라우팅에서 silent 누락"
            );
        }
        match MenuAction::StartDaemon {
            MenuAction::StartDaemon => assert_in_all(MenuAction::StartDaemon),
            MenuAction::StopDaemon => assert_in_all(MenuAction::StopDaemon),
            MenuAction::QuitApp => assert_in_all(MenuAction::QuitApp),
            MenuAction::ToggleAutostart => assert_in_all(MenuAction::ToggleAutostart),
        }
        assert_eq!(MenuAction::ALL.len(), 4, "variant 수 ↔ ALL 길이 불일치");
    }

    #[test]
    fn menu_ids_are_unique() {
        // If ids collide, click routing breaks.
        let ids: Vec<&str> = MenuAction::ALL.iter().map(|a| a.menu_id()).collect();
        let mut dedup = ids.clone();
        dedup.sort_unstable();
        dedup.dedup();
        assert_eq!(ids.len(), dedup.len(), "menu_id 중복: {ids:?}");
    }

    #[test]
    fn labels_are_unique_and_nonempty() {
        let labels: Vec<&str> = MenuAction::ALL.iter().map(|a| a.label()).collect();
        assert!(labels.iter().all(|l| !l.is_empty()), "빈 라벨: {labels:?}");
        let mut dedup = labels.clone();
        dedup.sort_unstable();
        dedup.dedup();
        assert_eq!(labels.len(), dedup.len(), "label 중복: {labels:?}");
    }

    // ── Target windows for show·hide ──
    // Pass the same canonical popout decision as production — with a separate decision for tests, this stays green even if the canonical one changes.
    use crate::commands::popout::is_popup_label;

    #[test]
    fn ui_windows_puts_popouts_first_and_main_last() {
        let labels = ["main", "slot-popup-1", "slot-popup-2"];
        assert_eq!(
            ui_windows(labels, is_popup_label),
            ["slot-popup-1", "slot-popup-2", "main"],
        );
    }

    #[test]
    fn ui_windows_keeps_main_last_wherever_it_arrives() {
        let labels = ["slot-popup-1", "main", "slot-popup-2"];
        assert_eq!(
            ui_windows(labels, is_popup_label).last(),
            Some(&"main"),
            "메인이 마지막이 아니면 뒤에 펼친 팝아웃이 포커스를 가져간다",
        );
    }

    #[test]
    fn ui_windows_excludes_unknown_windows() {
        let labels = ["main", "slot-popup-7", "devtools", "Main"];
        assert_eq!(ui_windows(labels, is_popup_label), ["slot-popup-7", "main"]);
    }

    #[test]
    fn ui_windows_without_main_returns_popouts_only() {
        let labels = ["devtools", "slot-popup-3"];
        assert_eq!(ui_windows(labels, is_popup_label), ["slot-popup-3"]);
    }

    #[test]
    fn ui_windows_orders_popouts_by_number_not_by_arrival() {
        let labels = ["slot-popup-10", "main", "slot-popup-2", "slot-popup-1"];
        assert_eq!(
            ui_windows(labels, is_popup_label),
            ["slot-popup-1", "slot-popup-2", "slot-popup-10", "main"],
        );
    }

    // An issued label carries a run marker — the order is decided by the sequence number after it (even if the marker is only digits).
    #[test]
    fn ui_windows_orders_issued_popouts_by_their_trailing_number() {
        use crate::commands::popout::popup_label;
        let (ten, two) = (popup_label(10), popup_label(2));
        let labels = [ten.as_str(), "main", two.as_str()];
        assert_eq!(
            ui_windows(labels, is_popup_label),
            [two.as_str(), ten.as_str(), "main"],
        );
        let digits = ["slot-popup-12345678-10", "slot-popup-12345678-9"];
        assert_eq!(
            ui_windows(digits, is_popup_label),
            ["slot-popup-12345678-9", "slot-popup-12345678-10"],
        );
    }

    #[test]
    fn ui_windows_order_does_not_depend_on_arrival_order() {
        let a = ["slot-popup-3", "slot-popup-1", "slot-popup-2", "main"];
        let b = ["main", "slot-popup-2", "slot-popup-3", "slot-popup-1"];
        assert_eq!(ui_windows(a, is_popup_label), ui_windows(b, is_popup_label));
    }

    #[test]
    fn ui_windows_puts_unnumbered_popouts_after_numbered_in_lexical_order() {
        // An empty tail · a non-numeric tail · a tail over u64 is treated as having no number.
        let labels = [
            "slot-popup-x",
            "slot-popup-99999999999999999999999",
            "slot-popup-",
            "slot-popup-9",
        ];
        assert_eq!(
            ui_windows(labels, is_popup_label),
            [
                "slot-popup-9",
                "slot-popup-",
                "slot-popup-99999999999999999999999",
                "slot-popup-x",
            ],
        );
    }

    #[test]
    fn ui_windows_breaks_equal_numbers_by_label() {
        // "007" and "7" are the same number — split by label so the order does not lean on arrival order.
        let a = ["slot-popup-7", "slot-popup-007"];
        let b = ["slot-popup-007", "slot-popup-7"];
        assert_eq!(
            ui_windows(a, is_popup_label),
            ["slot-popup-007", "slot-popup-7"]
        );
        assert_eq!(
            ui_windows(b, is_popup_label),
            ["slot-popup-007", "slot-popup-7"]
        );
    }

    #[test]
    fn ui_windows_main_only_and_empty() {
        assert_eq!(ui_windows(["main"], is_popup_label), ["main"]);
        assert!(ui_windows([], is_popup_label).is_empty());
        assert!(ui_windows(["devtools"], is_popup_label).is_empty());
    }

    #[test]
    fn grayscale_converts_color_to_gray_preserving_alpha() {
        let rgba = [
            200u8, 10, 30, 128, // Reddish, alpha=128
            10, 200, 30, 255, // Greenish, alpha=255
        ];
        let out = to_grayscale_rgba(&rgba, 2, 1);
        assert_eq!(out.len(), rgba.len(), "길이 보존");
        assert_eq!(out[0], out[1]);
        assert_eq!(out[1], out[2]);
        assert_eq!(out[3], 128, "alpha 보존(px0)");
        assert_eq!(out[4], out[5]);
        assert_eq!(out[5], out[6]);
        assert_eq!(out[7], 255, "alpha 보존(px1)");
        let expected0 = (0.299 * 200.0 + 0.587 * 10.0 + 0.114 * 30.0f32).round() as u8;
        assert_eq!(out[0], expected0, "px0 luma");
    }

    #[test]
    fn grayscale_pure_gray_input_is_idempotent_ish() {
        let rgba = [128u8, 128, 128, 255];
        let out = to_grayscale_rgba(&rgba, 1, 1);
        assert_eq!(out, vec![128, 128, 128, 255]);
    }

    #[test]
    fn grayscale_black_and_white_extremes() {
        let rgba = [0u8, 0, 0, 255, 255, 255, 255, 255];
        let out = to_grayscale_rgba(&rgba, 2, 1);
        assert_eq!(&out[0..4], &[0, 0, 0, 255]);
        assert_eq!(&out[4..8], &[255, 255, 255, 255]);
    }
}
