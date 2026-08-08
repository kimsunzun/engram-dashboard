//! tray core — 트레이 동작의 **순수 로직**(OS/GUI/네트워크 무의존).
//!
//! 순수성 불변식: tauri/discovery import 0 — 슬라이스/enum 만 다룬다(CLAUDE.md 「코어 격리」).
//! Launcher/DaemonProbe/dispatch 류 seam 은 **의도적 부재** — 통합 앱은 트레이 핸들러가
//! discovery command 를 직접 부르므로 불필요하다(ADR-0026, TRD §2).
//! 메뉴에 QuitTray 가 없는 건 트레이=앱 통합이라 무의미해서다(ADR-0026).
//! ★주의: show_main_ui/hide_main_ui actions·command 는 유지★ — ShowUi/HideUi 는 메뉴 *항목*만
//! 뺀 것이고(트레이 좌클릭이 대체), LLM/cdp 제어(CLAUDE.md §5)가 같은 actions 함수를 계속 쓴다.

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
