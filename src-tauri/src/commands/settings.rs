//! 셸 설정 · 창별 테마의 Tauri 어댑터 — 설정 command 넷 · 부팅 조회 command · 설정 알림 · 테마 밀기 · 부팅 정리.
//!
//! ★이 파일에 로직이 없다★: 설정 값·검증·저장은 `crate::settings` 가, 파일 위치·읽기·창별 해소·무엇을
//! 쓸어낼지는 `crate::ui_settings` 가 소유하고, 여기 남는 것은 Tauri 세계로의 번역(조회 응답 · **살아 있는
//! 웹뷰 세기** · `emit`/`emit_to` · **설정이 선언한 창 뽑기**)뿐이다(`commands/layout.rs` 와 같은 분담).
//! 설정 command 넷은 버스 `settings.*`(`layout::commands`)와 같은 서비스·같은 쓰기 진입점을 부르는 두 번째
//! 껍데기다(ADR-0081 결정 3).
//!
//! ## ★창별 테마를 읽는 자리가 둘인 이유★
//! - **부팅 = 프론트가 당긴다**(`get_ui_settings`). 창이 언제 스크립트를 다 올렸는지 셸이 모르므로
//!   밀면 첫 값이 유실된다 — 레이아웃이 겪은 그 레이스다(ADR-0102).
//! - **그 뒤 = 셸이 민다**(`ui.refresh` · `theme.default` 쓰기). 값이 언제 바뀌었는지는 프론트가 모른다.
//!
//! 둘 다 같은 [`EffectiveThemes`] 를 부르므로 답이 갈리지 않는다. 그리고 둘 다 **창 label 로 값을 고른다** —
//! 한쪽만 창을 알면 그 창은 부팅과 밀기에서 다른 테마를 본다.
// ADR-0167

use std::collections::BTreeSet;
use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager, State, Window};

use crate::settings::{
    reset_and_notify, set_and_notify, ResetOutcome, SetOutcome, SettingsEvents, SettingsSchema,
    SettingsService, SettingsSnapshot,
};
use crate::ui_settings::{
    sweep_dead_windows, write_atomic, EffectiveThemes, FileSource, LoadedTheme, ThemeWindows,
    UiSettingsPayload, UiSettingsRefresh,
};

/// 창마다 **자기 값**을 받는다 — 목적지를 지목해 보내므로 받는 쪽도 자기 label 로 구독해야 한다
/// (`src/theme/uiSettings.ts` · 사유 = `ui_settings::deliver_per_window`).
const EVT_UI_SETTINGS_UPDATED: &str = "ui:settings-updated";

/// 설정이 바뀌었다 — **모든 웹뷰에 같은 봉투**(설정은 창마다 다르지 않다). 짐 = [`SettingsSnapshot`]
/// (`rev` + 바뀐 키만). 창별 테마는 이 이벤트가 아니라 [`EVT_UI_SETTINGS_UPDATED`] 가 나른다.
const EVT_SETTINGS_CHANGED: &str = "settings:changed";

/// 부팅 조회 — 프론트가 창마다 한 번 당긴다.
///
/// ★창 label 을 인자로 받지 않는다★: 웹뷰가 스스로 밝히게 하면 잘못 적힌 label 하나가 남의 창 테마를
/// 가져간다. Tauri 가 넣어 주는 [`Window`] 가 그 값의 유일한 권위다(`commands/view_bus.rs` 의 같은 조항 —
/// 그 doc 이 **창** label 과 webview label 의 관계도 진다).
///
/// 답에 `source`(창 항목 파일을 썼나 / 못 써서 접혔나)가 함께 실린다 — 밀기와 **같은 페이로드 struct** 를
/// 쓰는 덕에 따로 배선하지 않았다.
///
/// ★밀기 순서 락을 여기서 잡지 않는다(의도)★: 이 조회는 부른 창 하나에만 답하고 아무것도 밀지 않으므로
/// **창들 사이의 순서**를 뒤집을 것이 없다. 여기에 락을 걸면 창 부팅이 남의 밀기 뒤에 줄만 선다.
///
/// 받는 쪽 빗장(`src/theme/uiSettings.ts` 의 `pushed`)이 덮는 것은 **한 방향뿐**이다 — 조회 답이 그보다
/// 새 알림을 덮는 것. 반대는 안 덮는다: 밀기가 "light" 를 읽고 멈춘 사이 값이 "e-ink" 로 바뀌고 새 창의
/// 조회가 "e-ink" 를 받아 그린 뒤, 그 밀기의 옛 알림이 도착해 그 창을 "light" 로 되돌린다.
/// ★그래도 결함이 아니다★ — 값을 바꾼 쪽이 저장 뒤에 다시 밀므로 그 밀기가 맞춘다. 파일이 상하는 경로가
/// 아니다. 고치려면 조회에도 세대를 실어야 하는데 그건 답 모양을 바꾼다.
#[tauri::command]
pub fn get_ui_settings(
    window: Window,
    themes: State<'_, Arc<EffectiveThemes>>,
) -> UiSettingsPayload {
    themes.for_window(window.label())
}

/// `settings.get` 의 사람 경로 — `key` = 정확한 키 또는 `.` 으로 끝나는 접두 · 빼면 전부.
/// 오류 문구는 서비스의 것 그대로다(종류 = 버스 쪽 코드 NOT_FOUND · INVALID_ARGUMENT · INTERNAL).
#[tauri::command]
pub fn settings_get(
    settings: State<'_, Arc<SettingsService>>,
    key: Option<String>,
) -> Result<SettingsSnapshot, String> {
    settings.get(key.as_deref()).map_err(|e| e.to_string())
}

/// `settings.set` 의 사람 경로. 실제로 바뀌었으면 [`EVT_SETTINGS_CHANGED`] 가 모든 웹뷰로 나가고,
/// `theme.default` 면 모든 창의 유효 테마도 다시 민다.
///
/// ★블로킹 풀에서 돈다★ — 이 쓰기는 디스크 `sync_all` 까지 기다린다. 동기 command 는 메인 스레드에서 돌아
/// 그동안 창 이벤트가 멈추고, async 본문에서 그대로 부르면 그 런타임 워커에 얹힌 다른 태스크가 멈춘다.
#[tauri::command]
pub async fn settings_set(
    app: AppHandle,
    settings: State<'_, Arc<SettingsService>>,
    themes: State<'_, Arc<EffectiveThemes>>,
    key: String,
    value: String,
) -> Result<SetOutcome, String> {
    let events = TauriSettingsEvents::new(app, Arc::clone(&themes));
    let settings = Arc::clone(&settings);
    off_the_runtime(move || set_and_notify(&settings, &events, &key, &value)).await
}

/// `settings.reset` 의 사람 경로 — 알림 · 블로킹 풀 사유는 [`settings_set`] 과 같다.
#[tauri::command]
pub async fn settings_reset(
    app: AppHandle,
    settings: State<'_, Arc<SettingsService>>,
    themes: State<'_, Arc<EffectiveThemes>>,
    key: String,
) -> Result<ResetOutcome, String> {
    let events = TauriSettingsEvents::new(app, Arc::clone(&themes));
    let settings = Arc::clone(&settings);
    off_the_runtime(move || reset_and_notify(&settings, &events, &key)).await
}

/// 설정 쓰기를 블로킹 풀에서 끝까지 돌린다. ★답보다 쓰기가 오래 산다★ — 기다리던 쪽이 사라져도 쓰기 ·
/// 알림은 끝까지 간다(값은 디스크에 남는다).
async fn off_the_runtime<T: Send + 'static>(
    write: impl FnOnce() -> Result<T, crate::settings::SettingsError> + Send + 'static,
) -> Result<T, String> {
    match tauri::async_runtime::spawn_blocking(write).await {
        Ok(outcome) => outcome.map_err(|e| e.to_string()),
        Err(e) => Err(format!("설정 쓰기가 끝나지 못했다: {e}")),
    }
}

/// `settings.schema` 의 사람 경로 — `key` 는 [`settings_get`] 과 같다.
#[tauri::command]
pub fn settings_schema(
    settings: State<'_, Arc<SettingsService>>,
    key: Option<String>,
) -> Result<SettingsSchema, String> {
    settings
        .schema(key.as_deref())
        .map(|items| SettingsSchema { items })
        .map_err(|e| e.to_string())
}

/// 앱 설정이 **선언한** 창 label 전량(`tauri.conf.json` 의 `app.windows`).
///
/// ★손 목록을 쓰지 않는다★ — 설정에 창이 늘면 이 함수가 함께 자란다. 런타임에 만드는 창(팝아웃)은 설정에
/// 없으므로 여기 안 든다: 부팅 쓸기가 지우는 것이 바로 그쪽이다.
///
/// 같은 설정 표를 읽는 형제가 하나 더 있다 — `view_commands::hidden_window_labels`(그쪽은 `visible` 로
/// 거른다). 둘이 재료를 공유한다는 것을 알고 남긴다.
// ADR-0167
pub fn declared_window_labels(app: &AppHandle) -> BTreeSet<String> {
    app.config()
        .app
        .windows
        .iter()
        .map(|window| window.label.clone())
        .collect()
}

/// 부팅 정리 — 죽은 창의 테마 항목을 지운다. ★`setup` 에서 한 번만 부른다★(사유·불변식 =
/// [`crate::ui_settings::sweep_dead_windows`], 원자성 = [`write_atomic`]).
///
/// 실패는 로그로 끝난다 — 이 정리는 앱이 뜨는 조건이 아니다.
// ADR-0167
pub fn sweep_dead_window_entries(app: &AppHandle) {
    let source = FileSource::in_data_dir();
    // 쓸 경로는 읽은 경로와 같아야 한다 — 두 번 고르면 데이터 폴더가 갈렸을 때 읽은 파일과 쓴 파일이 갈린다.
    let path = source.path().to_path_buf();
    sweep_dead_windows(&source, &declared_window_labels(app), |text| {
        write_atomic(&path, text)
    });
}

/// 테마 밀기의 창 쪽 — 살아 있는 웹뷰 명단과 창 하나로의 `emit_to`.
struct TauriThemeWindows<'a> {
    app: &'a AppHandle,
}

impl ThemeWindows for TauriThemeWindows<'_> {
    // ★명단은 Tauri 에서 받는다 — 레이아웃 명부가 아니다★. 저쪽은 `agent-tree` 를 모델 밖에 두므로
    //   (`layout::manager` 헤더) 그 명부로 세면 트리 창이 조용히 빠진다. 여기서 세는 것은 「지금 살아
    //   있는 웹뷰」이고, 그 label 이 웹뷰가 구독을 거는 값과 같다는 근거는 `view_commands` 의 같은 조항.
    fn labels(&self) -> Vec<String> {
        self.app.webview_windows().into_keys().collect()
    }

    // 밀기 락을 쥔 채 불린다 — Rust 쪽 수신자가 없어(웹뷰로만 나간다) 그 락을 되잡는 길이 없다.
    fn send(&self, label: &str, payload: UiSettingsPayload) -> Result<(), String> {
        self.app
            .emit_to(label, EVT_UI_SETTINGS_UPDATED, payload)
            .map_err(|e| e.to_string())
    }
}

/// `ui.refresh` 의 실물 — 창 항목 파일을 다시 읽어 모든 창의 유효 테마를 민다.
pub struct TauriUiSettings {
    app: AppHandle,
    themes: Arc<EffectiveThemes>,
}

impl TauriUiSettings {
    /// `themes` = 셸에 하나인 그 인스턴스(`lib.rs` 가 빌더에서 manage 한 것).
    pub fn new(app: AppHandle, themes: Arc<EffectiveThemes>) -> Self {
        Self { app, themes }
    }
}

impl UiSettingsRefresh for TauriUiSettings {
    // ★값만 민다★ — 창을 다시 만들거나 뷰를 갈아끼우지 않는다. 슬롯이 다시 마운트되면 챗은 컴포넌트
    //   상태라 대화가 영구 소실된다(ADR-0149).
    fn refresh(&self) -> Result<LoadedTheme, String> {
        self.themes
            .push_effective_themes(&TauriThemeWindows { app: &self.app })
    }
}

/// 설정 쓰기 알림의 실물 — 사람 경로(설정 command)와 LLM 경로(버스 표)가 같은 구현을 쓴다.
pub struct TauriSettingsEvents {
    app: AppHandle,
    themes: Arc<EffectiveThemes>,
}

impl TauriSettingsEvents {
    /// `themes` = 셸에 하나인 그 인스턴스 — 다른 인스턴스를 주면 두 밀기의 순서 락이 갈린다.
    pub fn new(app: AppHandle, themes: Arc<EffectiveThemes>) -> Self {
        Self { app, themes }
    }
}

// 둘 다 실패를 삼킨다(로그만) — 값은 이미 디스크와 메모리에 있고, 알림 유실은 그 쓰기를 되돌릴 사유가
//   아니다. 프론트는 `settings_get` · `get_ui_settings` 당기기로 따라잡는다.
impl SettingsEvents for TauriSettingsEvents {
    // 알림 순서 락 아래서 불린다(trait doc) — `emit` 은 웹뷰 스크립트 실행을 이벤트 루프에 넘기기만 하고 그
    //   실행을 기다리지 않는다.
    fn changed(&self, change: &SettingsSnapshot) {
        if let Err(e) = self.app.emit(EVT_SETTINGS_CHANGED, change) {
            tracing::warn!(
                module = "settings",
                event = EVT_SETTINGS_CHANGED,
                rev = change.rev,
                "설정 변경 알림을 못 보냈다: {e}"
            );
        }
    }

    fn theme_default_changed(&self) {
        // 사유는 밀기가 이미 로그로 남겼다.
        let _ = self
            .themes
            .push_effective_themes(&TauriThemeWindows { app: &self.app });
    }
}
