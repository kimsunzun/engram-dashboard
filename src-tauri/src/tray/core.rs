//! tray core — 트레이 동작의 **순수 로직**(OS/GUI/네트워크 무의존).
//!
//! ## 출처 (ADR-0026 1단계: 별도 tray-host crate 제거 + 순수 로직 살리기)
//! 구 `crates/engram-tray-host/src/core.rs` 에서 **순수 매핑/enum/픽셀 변환** 만 이관했다.
//! 버린 것: `LaunchError`/`DaemonProbe`/`Launcher`/`dispatch`/`causes_tray_exit`/`icon_state_from_probe`
//! (통합 앱은 트레이 핸들러가 discovery command 를 직접 부르므로 이 seam 불필요 — TRD §2).
//!
//! core.rs 순수성 유지 — tauri/discovery import 0(슬라이스/enum 만 다룬다, CLAUDE.md §4).
//!
//! ## ADR-0026 2단계: MenuAction 5개로 재정리(트레이=앱 통합)
//! 통합으로 "트레이 종료"(QuitTray)는 무의미해져 삭제(트레이=앱 → 트레이만 끄기 불가). 6→5:
//! StartDaemon/StopDaemon/ShowUi/HideUi/QuitApp. 라벨도 "UI 열기/닫기" → "UI 보이기/숨기기"
//! (창 show/hide 가 실제 동작 — destroy 가 아님), "완전 종료" → QuitApp.

// ── 메뉴 의도 ──────────────────────────────────────────────────────────────────

/// 트레이 메뉴 클릭이 표현하는 **의도**(렌더링/원천과 분리된 단일 enum).
/// 사람 클릭·LLM 호출·단축키가 모두 이 의도로 수렴한다(CLAUDE.md §5 손발/두뇌).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    /// "데몬 켜기" — 데몬 ensure(discovery WMI spawn). blocking → 워커.
    StartDaemon,
    /// "데몬 끄기" — 데몬만 graceful stop(UI 무관). blocking → 워커.
    StopDaemon,
    /// "UI 보이기" — main 창 show()+unminimize()+set_focus(). 프로세스 내부, IPC 없음.
    ShowUi,
    /// "UI 숨기기" — main 창 hide(). X=hide(prevent_close)와 같은 종착.
    HideUi,
    /// "완전 종료" — best-effort 데몬 graceful stop 후 app.exit(0). 진짜 종료는 이것뿐(ADR-0026).
    QuitApp,
}

impl MenuAction {
    /// 메뉴 항목의 안정 id(Tauri MenuItem 의 id 문자열로 사용).
    /// 디스플레이 라벨과 분리 — 라벨이 바뀌어도 id 는 불변(클릭 매핑 안정).
    pub const fn menu_id(self) -> &'static str {
        match self {
            MenuAction::StartDaemon => "start_daemon",
            MenuAction::StopDaemon => "stop_daemon",
            MenuAction::ShowUi => "show_ui",
            MenuAction::HideUi => "hide_ui",
            MenuAction::QuitApp => "quit_app",
        }
    }

    /// 메뉴 항목의 고정 라벨(상태 비반영).
    pub const fn label(self) -> &'static str {
        match self {
            MenuAction::StartDaemon => "데몬 켜기",
            MenuAction::StopDaemon => "데몬 끄기",
            MenuAction::ShowUi => "UI 보이기",
            MenuAction::HideUi => "UI 숨기기",
            MenuAction::QuitApp => "완전 종료",
        }
    }

    /// v2 메뉴에 노출되는 액션들(순서 = 표시 순서).
    /// 표시: 데몬 켜기, 데몬 끄기, UI 보이기, UI 숨기기, (구분선), 완전 종료.
    /// (구분선은 GUI shell 이 QuitApp 앞에 삽입 — core 는 액션만 안다.)
    pub const ALL: [MenuAction; 5] = [
        MenuAction::StartDaemon,
        MenuAction::StopDaemon,
        MenuAction::ShowUi,
        MenuAction::HideUi,
        MenuAction::QuitApp,
    ];
}

/// 메뉴 클릭 id → [`MenuAction`] 매핑(순수). 알 수 없는 id 면 None.
/// Tauri 의 MenuEvent.id 문자열을 받아 의도로 환원한다.
pub fn action_for_menu_id(id: &str) -> Option<MenuAction> {
    MenuAction::ALL.into_iter().find(|a| a.menu_id() == id)
}

// ── 상태 → 표시 매핑(순수) ─────────────────────────────────────────────────────────

/// 트레이 아이콘 상태. 데몬 생존을 시각화한다(활성=컬러/비활성=회색).
/// 실제 아이콘 두 벌은 GUI shell 이 들고, core 는 어떤 상태인지만 결정한다(렌더링 분리).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconState {
    /// 데몬 alive — 활성(컬러) 아이콘.
    Active,
    /// 데몬 없음/죽음 — 비활성(회색) 아이콘.
    Inactive,
}

/// 데몬 alive(bool) → [`IconState`] 매핑(순수).
pub fn icon_state_for(alive: bool) -> IconState {
    if alive {
        IconState::Active
    } else {
        IconState::Inactive
    }
}

// ── 아이콘 픽셀 변환(순수) ──────────────────────────────────────────────────────────

/// RGBA8 픽셀 버퍼를 desaturate(회색조)한 새 버퍼를 만든다(순수 — image 타입 무의존).
///
/// 비활성(데몬 죽음) 상태의 회색 아이콘을 컬러 원본에서 파생한다. luma = 0.299R+0.587G+0.114B
/// (Rec.601)로 각 RGB 채널을 동일 값으로 대체하고 alpha 는 보존한다 → R==G==B 인 무채색.
/// image 의존은 GUI shell 에만 두고 core 는 `&[u8]` 슬라이스만 받아 격리를 유지한다(CLAUDE.md §4).
///
/// `rgba.len()` 은 `w*h*4` 여야 한다(RGBA 4채널). 이 전제는 호출자(`image::into_rgba8()`)가
/// 보장한다. 어긋나면 디버그 빌드에서 panic(개발 계약 위반 조기 검출).
/// 릴리스에서 4의 배수가 아닌 잔여를 보존하지 **않는다** — 어차피 산출물의 유일 소비처
/// `Icon::from_rgba(_, w, h)` 가 `len==w*h*4` 를 요구해 잔여가 있으면 그쪽에서 Err→expect panic
/// 이라, 잔여를 살려도 "안전망"이 못 된다(전제 위반은 호출자 버그). 그래서 chunks_exact 의
/// 잔여는 버린다 — 전제가 지켜지면 잔여 자체가 없다.
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
        out.push(g); // R
        out.push(g); // G
        out.push(g); // B
        out.push(px[3]); // A 보존
    }
    out
}

// ── 테스트 (OS/GUI 무의존 순수 단위) ─────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_id_roundtrips_to_action() {
        // id ↔ action 매핑이 일관(각 액션의 id 로 다시 그 액션이 나온다) — 전부.
        for action in MenuAction::ALL {
            assert_eq!(action_for_menu_id(action.menu_id()), Some(action));
        }
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
        // ※ 새 variant 추가 시 여기 arm 을 추가해야 컴파일된다(강제 인지 지점).
        match MenuAction::StartDaemon {
            MenuAction::StartDaemon => assert_in_all(MenuAction::StartDaemon),
            MenuAction::StopDaemon => assert_in_all(MenuAction::StopDaemon),
            MenuAction::ShowUi => assert_in_all(MenuAction::ShowUi),
            MenuAction::HideUi => assert_in_all(MenuAction::HideUi),
            MenuAction::QuitApp => assert_in_all(MenuAction::QuitApp),
        }
        // 위 match 로 강제 인지된 variant 수와 ALL 길이가 일치하는지(중복/누락 동시 차단).
        assert_eq!(MenuAction::ALL.len(), 5, "variant 수 ↔ ALL 길이 불일치");
    }

    #[test]
    fn menu_ids_are_unique() {
        // id 충돌이면 클릭 라우팅이 깨진다 — 모두 distinct 보장.
        let ids: Vec<&str> = MenuAction::ALL.iter().map(|a| a.menu_id()).collect();
        let mut dedup = ids.clone();
        dedup.sort_unstable();
        dedup.dedup();
        assert_eq!(ids.len(), dedup.len(), "menu_id 중복: {ids:?}");
    }

    #[test]
    fn labels_are_unique_and_nonempty() {
        // 라벨이 모두 비지 않고 distinct(메뉴 표시 혼동 방지).
        let labels: Vec<&str> = MenuAction::ALL.iter().map(|a| a.label()).collect();
        assert!(labels.iter().all(|l| !l.is_empty()), "빈 라벨: {labels:?}");
        let mut dedup = labels.clone();
        dedup.sort_unstable();
        dedup.dedup();
        assert_eq!(labels.len(), dedup.len(), "label 중복: {labels:?}");
    }

    #[test]
    fn grayscale_converts_color_to_gray_preserving_alpha() {
        // 컬러 픽셀 2개(빨강 반투명, 초록 불투명) → R==G==B(무채색) + alpha 보존.
        let rgba = [
            200u8, 10, 30, 128, // 빨강 계열, alpha=128
            10, 200, 30, 255, // 초록 계열, alpha=255
        ];
        let out = to_grayscale_rgba(&rgba, 2, 1);
        assert_eq!(out.len(), rgba.len(), "길이 보존");
        // px0: 무채색 + alpha 보존.
        assert_eq!(out[0], out[1]);
        assert_eq!(out[1], out[2]);
        assert_eq!(out[3], 128, "alpha 보존(px0)");
        // px1: 무채색 + alpha 보존.
        assert_eq!(out[4], out[5]);
        assert_eq!(out[5], out[6]);
        assert_eq!(out[7], 255, "alpha 보존(px1)");
        // luma 값 검증(Rec.601): px0 = 0.299*200+0.587*10+0.114*30 ≈ 68.99 → 69.
        let expected0 = (0.299 * 200.0 + 0.587 * 10.0 + 0.114 * 30.0f32).round() as u8;
        assert_eq!(out[0], expected0, "px0 luma");
    }

    #[test]
    fn grayscale_pure_gray_input_is_idempotent_ish() {
        // 이미 무채색인 입력은 거의 그대로(반올림 오차 0): R==G==B 인 회색은 luma=그 값.
        let rgba = [128u8, 128, 128, 255];
        let out = to_grayscale_rgba(&rgba, 1, 1);
        assert_eq!(out, vec![128, 128, 128, 255]);
    }

    #[test]
    fn grayscale_black_and_white_extremes() {
        // 검정→검정, 흰색→흰색(clamp 경계 안전).
        let rgba = [0u8, 0, 0, 255, 255, 255, 255, 255];
        let out = to_grayscale_rgba(&rgba, 2, 1);
        assert_eq!(&out[0..4], &[0, 0, 0, 255]);
        assert_eq!(&out[4..8], &[255, 255, 255, 255]);
    }
}
