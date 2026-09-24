//! tray core — 트레이 동작의 **순수 로직**(OS/GUI/네트워크 무의존).
//!
//! 순수성 불변식: tauri/discovery import 0 — 슬라이스/enum 만 다룬다(CLAUDE.md 「코어 격리」).
//! Launcher/DaemonProbe/dispatch 류 seam 은 **의도적 부재** — 통합 앱은 트레이 핸들러가
//! discovery command 를 직접 부르므로 불필요하다(ADR-0026, TRD §2).
//! 메뉴에 QuitTray 가 없는 건 트레이=앱 통합이라 무의미해서다(ADR-0026).
//! ★주의: show_main_ui/hide_main_ui actions·command 는 유지★ — ShowUi/HideUi 는 메뉴 *항목*만
//! 뺀 것이고(트레이 좌클릭이 대체), LLM/cdp 제어(CLAUDE.md §5)가 같은 actions 함수를 계속 쓴다.

use crate::layout::MAIN_WINDOW_LABEL;

// ── 메뉴 의도 ──────────────────────────────────────────────────────────────────

// 트레이 메뉴 클릭이 표현하는 **의도**. 사람 클릭·LLM 호출·단축키가 모두 이 의도로
// 수렴한다(CLAUDE.md §5 손발/두뇌).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    StartDaemon,
    StopDaemon,
    QuitApp,
    ToggleAutostart,
}

impl MenuAction {
    // 안정 id — 라벨이 바뀌어도 불변(클릭 매핑 안정).
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

// ── 상태 → 표시 매핑(순수) ─────────────────────────────────────────────────────────

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

// ── 보이기·숨기기 대상 창(순수) ──────────────────────────────────────────────────────

// 트레이 보이기·숨기기가 다룰 창 = 팝아웃 전부(번호 오름차순) 뒤에 메인. 그 밖의 창은 뺀다 — 늘 숨어
// 있어야 하는 정적 창 `agent-tree` 가 여기 걸리면 화면에 떠 버린다(ADR-0225 가 걷을 예정).
// ★메인이 마지막인 것이 계약이다★: 호출자가 받은 순서대로 창을 펼치고 포커스를 주는데 그 셋이 모두 창을
//   활성화한다(tao 0.35 Windows = `SW_SHOW` · `SW_RESTORE` · `SetForegroundWindow`) — 메인 뒤에 다룬
//   팝아웃이 있으면 포커스를 그쪽이 가져간다. 같은 이유로 번호가 가장 큰 팝아웃이 메인 바로 아래에 온다.
// ★팝아웃을 받은 순서대로 두지 말 것★: 호출자가 넘기는 `webview_windows()` 는 부를 때마다 무작위 해셔의
//   새 HashMap 이라(tauri 2.11.3 `src/lib.rs:588-601`) 순서를 따르면 보이기마다 팝아웃 겹침 순서가 뒤섞인다.
// 팝아웃 판정은 인자로 받는다 — 테스트 밖 코드가 Tauri 에 묶인 `commands` 모듈에 기대지 않게 하려는 것이고,
//   테스트는 정본 판정(`commands::popout::is_popup_label`)을 그대로 넘겨 계약을 못 박는다.
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
    // 번호 있는 것 먼저(번호순), 번호 없는 것은 뒤에 라벨순. 같은 번호("7"·"007")도 라벨로 갈라 전순서를 만든다
    //   — 숫자·문자 비교를 섞는 비교자는 전순서가 아니라 정렬이 패닉할 수 있다.
    windows.sort_by_key(|&label| {
        let n = popout_number(label);
        (n.is_none(), n, label)
    });
    windows.extend(main);
    windows
}

// 번호 = 마지막 `-` 뒤 꼬리. 발급 라벨(`slot-popup-<n>`)에선 접두 뒤 꼬리와 같다 — 접두 상수는 Tauri 에 묶인
// `commands::popout` 에 있어 여기서 쓰지 않는다. 빈 꼬리·숫자 아님·u64 초과는 `None`.
fn popout_number(label: &str) -> Option<u64> {
    label.rsplit_once('-')?.1.parse().ok()
}

// ── 아이콘 픽셀 변환(순수) ──────────────────────────────────────────────────────────

// `rgba.len()` 은 `w*h*4` 여야 한다(RGBA 4채널). 이 전제는 호출자(Tauri `Image::from_bytes` 로
// 디코드한 `.rgba()`)가 보장한다 — 디코드 결과가 `(width, height)` 와 정합하는 길이의 버퍼다.
// len ≠ w*h*4 는 전부 계약 위반(4의 배수 여부 무관). debug 빌드는 아래 debug_assert 가 즉시
// panic 으로 잡고, 릴리스는 하류가 잡되 **soft** 하다 — 직접 소비처 `Image::new_owned(_, w, h)`
// 는 무검증 생성자지만, 그 `Image` 가 트레이에 닿기 전 `TryFrom → tray_icon::Icon::from_rgba`
// 의 길이 검증을 거친다(builder 경로 = `.ok()` 로 삼켜 아이콘 미설정 / `set_icon` 경로 = warn
// 로그 degrade — 어느 쪽도 panic 아님). 즉 debug_assert 는 유일한 방어선이 아니라 유일하게
// 즉시·시끄럽게 잡는 방어선이다. 전제가 지켜지면 chunks_exact 잔여는 없다.
pub fn to_grayscale_rgba(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    debug_assert_eq!(
        rgba.len(),
        (w as usize) * (h as usize) * 4,
        "to_grayscale_rgba: 버퍼 길이 ≠ w*h*4 (RGBA)"
    );
    let mut out = Vec::with_capacity(rgba.len());
    for px in rgba.chunks_exact(4) {
        // Rec.601 luma. f32 누적 후 반올림 — 정수 근사 누적오차 회피.
        let luma = 0.299 * px[0] as f32 + 0.587 * px[1] as f32 + 0.114 * px[2] as f32;
        let g = luma.round().clamp(0.0, 255.0) as u8;
        out.push(g);
        out.push(g);
        out.push(g);
        out.push(px[3]);
    }
    out
}

// ── 테스트 (OS/GUI 무의존 순수 단위) ─────────────────────────────────────────────────

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
        // ALL 누락 방지: 새 variant 를 추가하면 아래 exhaustive match 가 컴파일 에러를 내
        // (non-exhaustive) "이 variant 를 ALL 에 넣었는지" 를 강제 인지하게 한다.
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
        // id 충돌이면 클릭 라우팅이 깨진다.
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

    // ── 보이기·숨기기 대상 창 ──
    // 팝아웃 판정은 운영과 같은 정본을 넘긴다 — 테스트용 판정을 따로 두면 정본이 바뀌어도 여기가 초록이다.
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
    fn ui_windows_excludes_agent_tree_and_unknown_windows() {
        let labels = ["agent-tree", "main", "slot-popup-7", "devtools", "Main"];
        assert_eq!(ui_windows(labels, is_popup_label), ["slot-popup-7", "main"]);
    }

    #[test]
    fn ui_windows_without_main_returns_popouts_only() {
        let labels = ["agent-tree", "slot-popup-3"];
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

    #[test]
    fn ui_windows_order_does_not_depend_on_arrival_order() {
        let a = ["slot-popup-3", "slot-popup-1", "slot-popup-2", "main"];
        let b = ["main", "slot-popup-2", "slot-popup-3", "slot-popup-1"];
        assert_eq!(ui_windows(a, is_popup_label), ui_windows(b, is_popup_label));
    }

    #[test]
    fn ui_windows_puts_unnumbered_popouts_after_numbered_in_lexical_order() {
        // 빈 꼬리 · 숫자 아닌 꼬리 · u64 를 넘는 꼬리는 번호가 없는 것으로 본다.
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
        // "007" 과 "7" 은 같은 번호다 — 라벨로 갈라 순서가 도착 순서에 기대지 않게 한다.
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
        assert!(ui_windows(["agent-tree"], is_popup_label).is_empty());
    }

    #[test]
    fn grayscale_converts_color_to_gray_preserving_alpha() {
        let rgba = [
            200u8, 10, 30, 128, // 빨강 계열, alpha=128
            10, 200, 30, 255, // 초록 계열, alpha=255
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
