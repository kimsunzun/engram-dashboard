//! UI 설정의 Tauri 어댑터 — 부팅 조회 command 와 `ui.refresh` 가 미는 알림.
//!
//! ★이 파일에 로직이 없다★: 파일 위치·읽기·기본값은 `crate::ui_settings` 가 소유하고, 여기 남는 것은
//! Tauri 세계로의 번역 둘(조회 응답 · 전 창 emit)뿐이다(`commands/layout.rs` 와 같은 분담).
//!
//! ## ★읽는 자리가 둘인 이유★
//! - **부팅 = 프론트가 당긴다**(`get_ui_settings`). 창이 언제 스크립트를 다 올렸는지 셸이 모르므로
//!   밀면 첫 값이 유실된다 — 레이아웃이 겪은 그 레이스다(ADR-0102).
//! - **그 뒤 = 셸이 민다**(`ui.refresh`). 파일이 언제 바뀌었는지는 프론트가 모른다.
//!
//! 둘 다 같은 `load_theme` 을 부르므로 답이 갈리지 않는다.

use std::sync::Mutex;

use tauri::{AppHandle, Emitter};

use crate::ui_settings::{
    load_theme, FileSource, LoadedTheme, UiSettingsPayload, UiSettingsRefresh,
};

/// ★전 웹뷰가 받는다★ — main·팝아웃·트리가 **같은 값**을 본다. 창마다 다른 테마는 이 파일럿의 범위가
/// 아니고, 창을 골라 보내면 나중에 뜬 창이 옛 값으로 남는다(그 창은 부팅 조회로만 값을 받는다).
const EVT_UI_SETTINGS_UPDATED: &str = "ui:settings-updated";

/// 부팅 조회 — 프론트가 창마다 한 번 당긴다.
///
/// 답에 `source`(파일에서 왔나 / 기본값으로 접혔나)가 함께 실린다 — `ui.refresh` 와 **같은 페이로드
/// struct** 를 쓰는 덕에 따로 배선하지 않았다.
///
/// ★아래 순서 락을 여기서 잡지 않는다(의도)★: 이 조회는 부른 창 하나에만 답하고 아무것도 밀지 않으므로
/// **창들 사이의 순서**를 뒤집을 것이 없다. 여기에 락을 걸면 창 부팅이 남의 refresh 뒤에 줄만 선다.
///
/// 받는 쪽 빗장(`src/theme/uiSettings.ts` 의 `pushed`)이 덮는 것은 **한 방향뿐**이다 — 조회 답이 그보다
/// 새 알림을 덮는 것. 반대는 안 덮는다: refresh 가 "light" 를 읽고 멈춘 사이 파일이 "e-ink" 로 바뀌고
/// 새 창의 조회가 "e-ink" 를 받아 그린 뒤, 그 refresh 의 옛 알림이 도착해 전 창을 "light" 로 되돌린다.
/// ★그래도 결함이 아니다★ — 결과는 「낡았지만 모든 창이 같다」이고, 다음 refresh 가 맞춘다. 창별로 값이
/// 갈리거나 파일이 상하는 경로가 아니다. 고치려면 조회에도 세대를 실어야 하는데 그건 답 모양을 바꾼다.
#[tauri::command]
pub fn get_ui_settings() -> UiSettingsPayload {
    load_theme(&FileSource::in_data_dir()).into()
}

/// `ui.refresh` 의 실물.
pub struct TauriUiSettings {
    app: AppHandle,
    /// ★읽기부터 알림까지를 한 덩이로 묶는다★ — 사유는 [`TauriUiSettings::refresh`].
    gate: Mutex<()>,
}

impl TauriUiSettings {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            gate: Mutex::new(()),
        }
    }
}

impl UiSettingsRefresh for TauriUiSettings {
    // ★값만 민다★ — 창을 다시 만들거나 뷰를 갈아끼우지 않는다. 슬롯이 다시 마운트되면 챗은 컴포넌트
    //   상태라 대화가 영구 소실된다(ADR-0149).
    //
    // ★읽기와 알림 사이가 갈라지면 옛 값이 이긴다★: A 가 "light" 를 읽고 알림 전에 멈춘 사이 파일이
    //   "e-ink" 로 바뀌고 B 가 그것을 읽어 밀면, 그 뒤 깨어난 A 의 알림이 전 창을 "light" 로 되돌린다.
    //   화면은 최신 파일과 어긋난 채로 남고 A 의 성공 답장은 화면에 대한 진술이 아니게 된다. 그래서
    //   읽기~알림을 이 락으로 직렬화한다 — 마지막 알림 = 마지막 읽기.
    //   ★락 보유 중 부르는 외부 호출은 `emit` 하나★이고 그것은 이 락을 되잡지 않는다(Rust 쪽 수신자가
    //   없다 — 웹뷰로만 나간다). 다른 락과 겹치는 순서도 없다(이 락은 이 구조체 것뿐이다).
    fn refresh(&self) -> Result<LoadedTheme, String> {
        // 락이 중독돼도(보유 중 패닉) 계속 돈다 — 이 락이 지키는 것은 순서뿐이라 뒤에 깨질 상태가 없다.
        // 여기서 unwrap 하면 한 번의 패닉이 이후 모든 refresh 를 영구히 막는다.
        let _order = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        // ★경로를 캐시하지 않고 매번 다시 고른다★ — 부팅 조회와 같은 함수를 같은 방식으로 불러야 두 읽기
        //   지점이 반드시 같은 파일을 본다.
        let loaded = load_theme(&FileSource::in_data_dir());
        let payload: UiSettingsPayload = loaded.into();
        // ★못 보냈으면 성공이 아니다★ — 이 명령의 결과는 알림이 전부다(사유 = 포트 trait 의 doc).
        //   로그도 함께 남긴다: 답장은 명령을 낸 쪽만 보고, 이 셸의 로그를 읽는 사람은 그 답장을 못 본다.
        self.app
            .emit(EVT_UI_SETTINGS_UPDATED, payload)
            .map_err(|e| {
                tracing::warn!(
                    module = "ui_settings",
                    event = EVT_UI_SETTINGS_UPDATED,
                    "UI 설정 알림을 못 보냈다 — 어느 창에도 안 닿았다: {e}"
                );
                format!("UI 설정 알림을 못 보냈다(어느 창에도 안 닿았다): {e}")
            })?;
        Ok(loaded)
    }
}
