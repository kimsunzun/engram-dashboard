//! 창 테마 — 값([`UiTheme`]) · 전역 테마([`global_theme`] — 설정 `theme.default`) · **창마다의 유효 테마**
//! ([`EffectiveThemes`]) · 창마다 배달([`deliver_per_window`]) · 읽기·쓰기·밀기 손잡이([`ThemeControl`]).
//!
//! - **창 테마의 집은 화면 상태 모델이다** — 레이아웃 창은 `ViewManager` 의 창 항목(`WindowAttrs::theme`), 트리
//!   창(`agent-tree`)은 [`TreeAttrs`]. 창과 같이 살고 같이 죽으며 기록기가 `state.json` 에 싣는다(TRD
//!   S21-storage §6-3).
//! - ★유효 값은 [`EffectiveThemes`] 한 자리에서만 계산한다★ — 부팅 당기기(`get_ui_settings`) · 밀기
//!   (`ui:settings-updated`) · `window.getTheme` 이 같은 출처를 본다. 한쪽만 다른 출처를 보면 그 창은 부팅과 밀기에서
//!   다른 테마를 받는다.
//! - ★락 순서 = 테마 관문 › {설정 상태 락 · `ViewManager` 락 · 트리 칸 락}★ — 관문만 바깥이고 나머지 셋은 하나씩
//!   짧게 잡고 놓는다(겹쳐 잡지 않는다 — TRD §6-3). 테마를 바꾸는 쪽은 저장을 끝내고 모든 락을 놓은 뒤 밀기를 부른다.
//! - 보내는 자리는 호출자가 넣는다([`ThemeWindows`]) — Tauri 를 이 모듈에 들이지 않는다.
// ADR-0265

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use engram_dashboard_base::sync;

use crate::layout::LayoutState;
use crate::settings::{SettingsService, THEME_DEFAULT};
use crate::state::convert::TREE_WINDOW_ID;
use crate::state::tree_attrs::TreeAttrs;

/// 설정 `theme.default` 의 값이 [`UiTheme`] 로 안 읽힐 때 쓰는 값 — 표의 선택지와 [`UiTheme::as_wire`] 철자가
/// 같으면 닿지 않는다(이 모듈의 시험이 둘을 맞댄다).
pub const DEFAULT_THEME: UiTheme = UiTheme::Dark;

/// 화면 테마 — 세 값이 전부다.
///
/// ★`EInk` 를 빼지 말 것★: e-ink 는 밝기 변형이 아니라 **색을 무력화하는** 별도 의도라 dark/light 로
/// 접히지 않는다(ADR-0062).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiTheme {
    Dark,
    Light,
    EInk,
}

impl UiTheme {
    /// 프론트가 `document.documentElement` 의 `data-theme` 에 그대로 박는 문자열.
    ///
    /// ★`src/styles/theme.css` 의 `:root[data-theme='…']` 셀렉터와 **같은 철자**여야 한다★ — 어긋나면
    /// 오류 하나 없이 스타일만 안 붙어서, 화면만 보고는 철자가 틀린 것인지 파일이 안 읽힌 것인지 모른다.
    pub const fn as_wire(self) -> &'static str {
        match self {
            UiTheme::Dark => "dark",
            UiTheme::Light => "light",
            UiTheme::EInk => "e-ink",
        }
    }

    /// [`Self::as_wire`] 의 역 — 그 세 철자만 받는다(대소문자를 가린다).
    pub fn from_wire(raw: &str) -> Option<Self> {
        match raw {
            "dark" => Some(UiTheme::Dark),
            "light" => Some(UiTheme::Light),
            "e-ink" => Some(UiTheme::EInk),
            _ => None,
        }
    }
}

/// 설정의 전역 테마. 설정 값이 [`UiTheme`] 철자가 아니면 [`DEFAULT_THEME`] 이다(닿지 않는 자리 —
/// [`DEFAULT_THEME`] 의 doc).
pub fn global_theme(settings: &SettingsService) -> UiTheme {
    settings
        .effective(THEME_DEFAULT)
        .as_deref()
        .and_then(UiTheme::from_wire)
        .unwrap_or(DEFAULT_THEME)
}

/// 창 하나의 테마 — `own` = 그 창에만 정한 값(`None` = 정하지 않았다 · 전역을 따른다) · `effective` = 그 창에
/// 칠하는 값.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowTheme {
    pub own: Option<UiTheme>,
    pub effective: UiTheme,
}

impl WindowTheme {
    /// 유효 테마 = 그 창의 테마 ?? 전역 테마(TRD S21-storage §5-6).
    // ADR-0265
    pub fn resolve(own: Option<UiTheme>, global: UiTheme) -> Self {
        WindowTheme {
            own,
            effective: own.unwrap_or(global),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ThemeError {
    /// 레이아웃 모델에도 없고 트리 창도 아니다.
    #[error("window 없음: {0} — 창 label 은 window.list 가 주고, 트리 창은 agent-tree 다")]
    UnknownWindow(String),
    /// 쓰기만 낸다 — 읽기는 독 든 모델에서도 테마 값 하나를 읽는다(반쯤 바뀐 모델에도 해롭지 않다).
    #[error("레이아웃 락에 독이 들어 창 테마를 못 썼다")]
    Poisoned,
    /// 썼는데 **지목한 그 창이** 알림을 못 받았다([`ThemeControl::set`]) — 쓴 값은 남는다(되돌리지 않는다).
    /// `reason` = 그 창의 실패 사유 · `others_failed` = 같은 밀기에서 못 받은 다른 창 수.
    #[error(
        "{window} 창 테마는 정했지만(window.getTheme 이 새 값을 답한다) 그 창에 못 보냈다: {reason}{}",
        others_note(.others_failed)
    )]
    Undelivered {
        window: String,
        reason: String,
        others_failed: usize,
    },
}

fn others_note(others_failed: &usize) -> String {
    match *others_failed {
        0 => String::new(),
        n => format!(" — 다른 창 {n}개도 못 받았다"),
    }
}

/// 프론트로 나가는 값 — 부팅 조회(`get_ui_settings`)와 밀기(`ui:settings-updated`)가 **같은 모양**을 쓴다.
///
/// 필드 이름은 wire 계약이다 — 프론트는 `theme` 만 읽는다(`src/theme/uiSettings.ts`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct UiSettingsPayload {
    pub theme: String,
}

impl From<UiTheme> for UiSettingsPayload {
    fn from(theme: UiTheme) -> Self {
        UiSettingsPayload {
            theme: theme.as_wire().to_string(),
        }
    }
}

/// 창 명단과 창 하나로의 배달 — 운영 = Tauri 어댑터(`commands::settings`), 시험 = 가짜(ADR-0012).
///
/// ★[`EffectiveThemes::push_effective_themes`] 가 테마 관문을 쥔 채 부른다★ — 구현이 그 함수를 다시 부르면
/// 교착이다.
pub trait ThemeWindows: Send + Sync {
    /// 지금 살아 있는 웹뷰 창 label 전량.
    fn labels(&self) -> Vec<String>;
    /// 그 창 하나에만 보낸다.
    fn send(&self, label: &str, payload: UiSettingsPayload) -> Result<(), String>;
}

/// 실패 문구에 펼치는 창 수 상한 — 창이 많을 때 문구 · 로그가 창 수만큼 자라지 않게.
const MAX_FAILED_DETAILS: usize = 3;

/// 알림을 못 받은 창 하나 — `reason` = [`ThemeWindows::send`] 가 준 사유 그대로.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedWindow {
    pub label: String,
    pub reason: String,
}

/// [`deliver_per_window`] 의 실패 — 「알림이 안 닿았다」(ADR-0166 결정 6).
///
/// ★못 받은 창을 문구가 아니라 값으로 나른다★ — [`ThemeControl::set`] 이 지목한 창이 그중에 있나를 여기서 가른다.
/// 사람이 읽는 문구(`Display` — 앞 [`MAX_FAILED_DETAILS`] 창은 사유까지 · 나머지는 수로 뭉친다)는 가장자리(로그
/// 한 줄 · 명령 오류)에서만 만든다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Undelivered {
    /// 보낼 창이 하나도 없다.
    NoWindows,
    /// 시도한 `tried` 창 가운데 못 받은 창 전량 — 시도 순이고 비지 않는다.
    Refused {
        tried: usize,
        failed: Vec<FailedWindow>,
    },
}

impl Undelivered {
    /// 못 받은 창 전량 — [`Self::NoWindows`] 면 비었다(시도한 창이 없다).
    pub fn failed(&self) -> &[FailedWindow] {
        match self {
            Undelivered::NoWindows => &[],
            Undelivered::Refused { failed, .. } => failed,
        }
    }
}

impl fmt::Display for Undelivered {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (tried, failed) = match self {
            Undelivered::NoWindows => return f.write_str("알림을 받을 창이 하나도 없다"),
            Undelivered::Refused { tried, failed } => (tried, failed),
        };
        write!(f, "{}/{tried} 창에 못 보냈다 — ", failed.len())?;
        for (i, window) in failed.iter().take(MAX_FAILED_DETAILS).enumerate() {
            if i > 0 {
                f.write_str(" · ")?;
            }
            write!(f, "{}: {}", window.label, window.reason)?;
        }
        if failed.len() > MAX_FAILED_DETAILS {
            write!(f, " · 외 {}개", failed.len() - MAX_FAILED_DETAILS)?;
        }
        Ok(())
    }
}

/// 창마다 **그 창의 값**을 보낸다 — `targets` = (창 label, 그 창의 유효 테마), 보내는 자리는 호출자가 넣는다.
///
/// ★한 봉투를 전 창에 뿌리지 않는다★: 창마다 값이 다를 수 있으므로 목적지를 지목해 보내야 한다. 받는 쪽도
/// 자기 label 로 구독해야 한다 — Tauri 는 `Any` 로 등록된 리스너를 **필터와 무관하게 전부** 깨우고 JS
/// `listen()` 의 기본 타깃이 그 `Any` 다(`view_commands::TauriViewDispatch` 가 같은 쌍을 진다).
///
/// `Err` 는 [`Undelivered`] 의 두 갈래다. ★한 창이 못 받아도 남은 창까지 다 시도하고 나서 실패로 답한다★ — 죽은
/// 창 하나에서 멈추면 나머지 창이 옛 값으로 남고, 그 답은 실패라 **어느 창이 갱신됐는지 아무도 모른다.**
pub fn deliver_per_window<E>(targets: &[(String, UiTheme)], mut emit: E) -> Result<(), Undelivered>
where
    E: FnMut(&str, UiSettingsPayload) -> Result<(), String>,
{
    if targets.is_empty() {
        return Err(Undelivered::NoWindows);
    }

    let failed: Vec<FailedWindow> = targets
        .iter()
        .filter_map(|(label, theme)| {
            emit(label, (*theme).into())
                .err()
                .map(|reason| FailedWindow {
                    label: label.clone(),
                    reason,
                })
        })
        .collect();
    if failed.is_empty() {
        return Ok(());
    }
    Err(Undelivered::Refused {
        tried: targets.len(),
        failed,
    })
}

/// 창마다 **유효 테마 = 그 창의 테마 ?? 설정 `theme.default`** 를 계산하는 단일 자리(TRD S21-storage §5-6) — 창
/// 테마를 쓰는 길도 여기 하나다(트리 창 = [`TreeAttrs`] · 그 밖 = `ViewManager`).
///
/// ★셸에 하나만 둔다★ — 밀기 순서를 지키는 관문 락이 이 안에 있어서, 인스턴스가 둘이면 두 밀기가 서로를 못 본다.
/// 설정 · 모델 · 트리 칸도 셸에 하나인 그 인스턴스여야 한다 — 다른 모델을 주면 쓰기와 밀기가 다른 창 테마를 본다.
pub struct EffectiveThemes {
    settings: Arc<SettingsService>,
    layout: LayoutState,
    tree: Arc<TreeAttrs>,
    /// ★읽기부터 알림까지를 한 덩이로 묶는다★ — 사유는 [`Self::push_effective_themes`].
    gate: Mutex<()>,
}

impl EffectiveThemes {
    pub fn new(settings: Arc<SettingsService>, layout: LayoutState, tree: Arc<TreeAttrs>) -> Self {
        Self {
            settings,
            layout,
            tree,
            gate: Mutex::new(()),
        }
    }

    /// 창 하나의 테마 — `window.getTheme` 의 답. 관문을 잡지 않는다(밀지 않으므로 창들 사이의 순서를 뒤집을 것이
    /// 없다).
    pub fn window_theme(&self, window: &str) -> Result<WindowTheme, ThemeError> {
        let own = self.own_theme(window)?;
        Ok(WindowTheme::resolve(own, global_theme(&self.settings)))
    }

    /// 부팅 당기기 — 창 하나에 답하고 아무것도 밀지 않는다. ★모르는 창에도 답한다(전역 값)★ — 모델에 들기 전에
    /// 스크립트를 올린 창(막 만들어지는 팝아웃)도 첫 값을 받아야 하고, 그런 창은 아직 자기 테마가 없다.
    pub fn for_window(&self, window: &str) -> UiSettingsPayload {
        let own = self.own_theme(window).ok().flatten();
        WindowTheme::resolve(own, global_theme(&self.settings))
            .effective
            .into()
    }

    /// 창 하나의 테마를 쓴다 — `None` = 그 창의 테마를 지운다(전역을 따른다). ★밀지 않는다★ — 미는 것은
    /// [`ThemeControl::set`]. 레이아웃 창은 `version` 이 아니라 `attrs_rev` 가 오른다(바뀌었을 때만).
    pub fn set_window_theme(
        &self,
        window: &str,
        theme: Option<UiTheme>,
    ) -> Result<WindowTheme, ThemeError> {
        if window == TREE_WINDOW_ID {
            self.tree.set_theme(theme);
        } else {
            let mut mgr = self.layout.0.lock().map_err(|_| ThemeError::Poisoned)?;
            mgr.set_window_theme(window, theme)
                .map_err(|_| ThemeError::UnknownWindow(window.to_string()))?;
        }
        Ok(WindowTheme::resolve(theme, global_theme(&self.settings)))
    }

    /// 모든 창의 유효 값을 새로 계산해 창마다 민다. `Err` = [`deliver_per_window`] 의 그것 — warn 로그도 여기서
    /// 남긴다(부르는 쪽이 다시 남기지 않는다).
    ///
    /// ★읽기와 알림 사이가 갈라지면 옛 값이 이긴다★: A 가 "light" 를 읽고 알림 전에 멈춘 사이 값이 "e-ink" 로
    /// 바뀌고 B 가 그것을 읽어 밀면, 그 뒤 깨어난 A 의 알림이 창들을 "light" 로 되돌린다. 그래서 읽기~알림을 관문
    /// 락으로 직렬화한다 — 마지막 알림 = 마지막 읽기. 쓰는 쪽은 저장을 **끝낸 뒤** 이 함수를 부르므로, 마지막 밀기는
    /// 마지막 쓰기를 본다.
    ///
    /// ★관문 아래에서 설정 상태 락 · 창 명단 · 트리 칸 락 · `ViewManager` 락을 하나씩 짧게 잡고 놓은 뒤 보낸다★ —
    /// 셋 중 어느 것도 겹쳐 잡지 않고, 보낼 때는 관문만 쥔다. 부르는 쪽은 아무 락도 쥐지 않는다.
    // ADR-0265
    pub fn push_effective_themes(&self, windows: &dyn ThemeWindows) -> Result<(), Undelivered> {
        // 락이 중독돼도(보유 중 패닉) 계속 돈다 — 이 락이 지키는 것은 순서뿐이라 뒤에 깨질 상태가 없다.
        // 여기서 unwrap 하면 한 번의 패닉이 이후 모든 밀기를 영구히 막는다.
        let _order = sync::lock(&self.gate);
        let global = global_theme(&self.settings);
        let labels = windows.labels();
        let tree = self.tree.attrs().theme;
        let model: HashMap<&str, Option<UiTheme>> = {
            let mgr = self.layout.0.lock().unwrap_or_else(PoisonError::into_inner);
            labels
                .iter()
                .filter_map(|label| {
                    let attrs = mgr.window_attrs(label).ok()?;
                    Some((label.as_str(), attrs.theme))
                })
                .collect()
        };
        let targets: Vec<(String, UiTheme)> = labels
            .iter()
            .map(|label| {
                let own = if label == TREE_WINDOW_ID {
                    tree
                } else {
                    model.get(label.as_str()).copied().flatten()
                };
                (label.clone(), WindowTheme::resolve(own, global).effective)
            })
            .collect();
        deliver_per_window(&targets, |label, payload| windows.send(label, payload)).inspect_err(
            |undelivered| {
                tracing::warn!(
                    module = "theme",
                    "창별 테마 알림을 못 보냈다: {undelivered}"
                );
            },
        )
    }

    // 읽기는 독 든 모델에서도 한다 — 테마 값 하나라 반쯤 바뀐 모델에도 해롭지 않다(`state::boot_plugin` 의
    //   `LiveSource::revision` 과 같은 판단).
    fn own_theme(&self, window: &str) -> Result<Option<UiTheme>, ThemeError> {
        if window == TREE_WINDOW_ID {
            return Ok(self.tree.attrs().theme);
        }
        let mgr = self.layout.0.lock().unwrap_or_else(PoisonError::into_inner);
        mgr.window_attrs(window)
            .map(|attrs| attrs.theme)
            .map_err(|_| ThemeError::UnknownWindow(window.to_string()))
    }
}

/// 테마를 읽고 · 쓰고 · 미는 손잡이 — 셸에 하나인 [`EffectiveThemes`] 와 창 쪽 포트를 묶는다. 명령 표 · 설정 알림 ·
/// 복원 조율자 · 부팅이 같은 것을 쥔다.
///
/// ★알림 유실은 쓰기를 되돌리지 않는다★ — 값은 이미 모델(저장이 켜진 실행이면 기록기를 거쳐 `state.json` 에도)에 있고, 창은 다음
/// 당기기 · 밀기로 따라잡는다. 답이 갈리는 것은 [`Self::set`] 의 지목한 창 하나뿐이고 나머지 실패는 로그로 끝난다.
#[derive(Clone)]
pub struct ThemeControl {
    themes: Arc<EffectiveThemes>,
    windows: Arc<dyn ThemeWindows>,
}

impl ThemeControl {
    pub fn new(themes: Arc<EffectiveThemes>, windows: Arc<dyn ThemeWindows>) -> Self {
        Self { themes, windows }
    }

    pub fn get(&self, window: &str) -> Result<WindowTheme, ThemeError> {
        self.themes.window_theme(window)
    }

    /// 그 창의 테마를 쓰고 모든 창을 다시 민다. ★아무 락도 쥐지 않고 부른다★.
    ///
    /// [`ThemeError::Undelivered`] = 썼는데 **그 창이** 못 받았다(쓴 값은 남는다). 다른 창만 못 받았으면 성공이다.
    /// 그 창이 살아 있는 웹뷰 명단([`ThemeWindows::labels`])에 없어도 성공이다 — 아직 안 뜬 창은 뜰 때 이 값을
    /// 당겨 간다([`EffectiveThemes::for_window`]).
    pub fn set(&self, window: &str, theme: Option<UiTheme>) -> Result<WindowTheme, ThemeError> {
        let written = self.themes.set_window_theme(window, theme)?;
        if let Err(undelivered) = self.themes.push_effective_themes(self.windows.as_ref()) {
            let failed = undelivered.failed();
            // ADR-0166 결정 6: 알림이 못 나가면 성공으로 답하지 않는다 — 이 답이 말하는 창은 지목한 그 창이다.
            if let Some(own) = failed.iter().find(|missed| missed.label == window) {
                return Err(ThemeError::Undelivered {
                    window: window.to_string(),
                    reason: own.reason.clone(),
                    others_failed: failed.len() - 1,
                });
            }
        }
        Ok(written)
    }

    /// 모든 창에 유효 테마를 민다. ★아무 락도 쥐지 않고 부른다★.
    pub fn push(&self) {
        // 실패는 밀기가 로그로 남겼다 — 이 길의 부르는 쪽(부팅 · 복원 수락 · `theme.default` 알림)은 배달로 답을
        //   가르지 않는다(타입 doc).
        let _ = self.themes.push_effective_themes(self.windows.as_ref());
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::layout::MAIN_WINDOW_LABEL;

    const POPUP: &str = "slot-popup-1";
    /// [`Recording`] 이 받지 못하는 창에 주는 사유.
    const CLOSED: &str = "창이 이미 닫혔다";

    /// 시험 하나 몫의 설정 폴더 — 셸 `setup` 처럼 쓰기를 연다. 끝나면 치운다.
    struct Config(PathBuf);

    impl Config {
        fn new(tag: &str) -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            Config(std::env::temp_dir().join(format!(
                "engram-theme-{tag}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            )))
        }

        fn service(&self) -> Arc<SettingsService> {
            let service = SettingsService::load_from_dir(&self.0);
            service.enable_writes();
            Arc::new(service)
        }
    }

    impl Drop for Config {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    /// 창 명단을 정해 두고 보낸 것을 순서대로 남긴다. `refuse` = 받지 못하는 창.
    #[derive(Default)]
    struct Recording {
        labels: Vec<String>,
        refuse: Vec<String>,
        sent: Mutex<Vec<(String, String)>>,
    }

    impl Recording {
        fn new(labels: &[&str]) -> Self {
            Recording {
                labels: labels.iter().map(|label| label.to_string()).collect(),
                ..Self::default()
            }
        }

        fn refusing(labels: &[&str], refuse: &[&str]) -> Self {
            Recording {
                refuse: refuse.iter().map(|label| label.to_string()).collect(),
                ..Self::new(labels)
            }
        }

        fn take(&self) -> Vec<(String, String)> {
            std::mem::take(&mut *self.sent.lock().unwrap())
        }
    }

    impl ThemeWindows for Recording {
        fn labels(&self) -> Vec<String> {
            self.labels.clone()
        }

        fn send(&self, label: &str, payload: UiSettingsPayload) -> Result<(), String> {
            self.sent
                .lock()
                .unwrap()
                .push((label.to_string(), payload.theme));
            if self.refuse.iter().any(|refused| refused == label) {
                return Err(CLOSED.to_string());
            }
            Ok(())
        }
    }

    fn sent(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(label, theme)| (label.to_string(), theme.to_string()))
            .collect()
    }

    struct Rig {
        _config: Config,
        settings: Arc<SettingsService>,
        layout: LayoutState,
        tree: Arc<TreeAttrs>,
        themes: EffectiveThemes,
    }

    /// main + 팝아웃 하나가 모델에 있다.
    fn rig(tag: &str) -> Rig {
        let config = Config::new(tag);
        let settings = config.service();
        let layout = LayoutState::new();
        layout.0.lock().unwrap().create_window(POPUP).unwrap();
        let tree = Arc::new(TreeAttrs::default());
        let themes = EffectiveThemes::new(settings.clone(), layout.clone(), tree.clone());
        Rig {
            _config: config,
            settings,
            layout,
            tree,
            themes,
        }
    }

    /// 세 값이 다 살아 있어야 한다 — e-ink 를 dark/light 로 접으면 그 테마의 의도(색 무력화)가 사라진다(ADR-0062).
    #[test]
    fn every_theme_name_round_trips() {
        for (raw, expected) in [
            ("dark", UiTheme::Dark),
            ("light", UiTheme::Light),
            ("e-ink", UiTheme::EInk),
        ] {
            assert_eq!(UiTheme::from_wire(raw), Some(expected));
            // 프론트가 `data-theme` 에 박는 철자 = `src/styles/theme.css` 의 셀렉터.
            assert_eq!(expected.as_wire(), raw);
        }
    }

    /// ★설정 표의 선택지와 이 enum 의 철자가 같다★ — 어긋나면 `theme.default` 의 그 값이 화면에서 조용히
    /// [`DEFAULT_THEME`] 로 접힌다(설정 답은 성공인데 화면은 안 바뀐다).
    #[test]
    fn every_theme_default_choice_is_a_ui_theme_spelling() {
        let config = Config::new("choices");
        let schema = config
            .service()
            .schema(Some(THEME_DEFAULT))
            .expect("표에 있는 키");
        let choices = schema[0].choices.clone().expect("choice 키");
        assert_eq!(choices.len(), 3);
        for choice in &choices {
            assert!(UiTheme::from_wire(choice).is_some(), "{choice}");
        }
        assert_eq!(schema[0].default, DEFAULT_THEME.as_wire());
    }

    #[test]
    fn the_effective_theme_is_the_windows_own_else_the_global() {
        assert_eq!(
            WindowTheme::resolve(Some(UiTheme::EInk), UiTheme::Light).effective,
            UiTheme::EInk
        );
        assert_eq!(
            WindowTheme::resolve(None, UiTheme::Light),
            WindowTheme {
                own: None,
                effective: UiTheme::Light
            }
        );
    }

    #[test]
    fn a_window_without_its_own_theme_follows_theme_default() {
        let rig = rig("follow");
        assert_eq!(
            rig.themes.window_theme(MAIN_WINDOW_LABEL),
            Ok(WindowTheme {
                own: None,
                effective: DEFAULT_THEME
            })
        );
        rig.settings.set(THEME_DEFAULT, "light").unwrap();
        assert_eq!(
            rig.themes
                .window_theme(MAIN_WINDOW_LABEL)
                .unwrap()
                .effective,
            UiTheme::Light
        );
    }

    #[test]
    fn clearing_a_windows_theme_falls_back_to_the_global() {
        let rig = rig("clear");
        rig.settings.set(THEME_DEFAULT, "light").unwrap();
        let set = rig
            .themes
            .set_window_theme(POPUP, Some(UiTheme::EInk))
            .unwrap();
        assert_eq!(set.effective, UiTheme::EInk);

        let cleared = rig.themes.set_window_theme(POPUP, None).unwrap();

        assert_eq!(
            cleared,
            WindowTheme {
                own: None,
                effective: UiTheme::Light
            }
        );
        assert_eq!(rig.themes.window_theme(POPUP), Ok(cleared));
    }

    /// 트리 창은 레이아웃 모델 밖이다 — 그 창의 값은 트리 칸에, 나머지는 `ViewManager` 의 창 항목에 들고 번호도 각자의
    /// 것이 오른다(레이아웃 `version` 은 그대로).
    #[test]
    fn the_tree_window_writes_the_tree_cell_and_the_others_write_the_model() {
        let rig = rig("route");
        let (version, attrs_rev) = {
            let mgr = rig.layout.0.lock().unwrap();
            (mgr.version, mgr.attrs_rev())
        };
        let tree_rev = rig.tree.rev();

        rig.themes
            .set_window_theme(TREE_WINDOW_ID, Some(UiTheme::Light))
            .unwrap();
        assert_eq!(rig.tree.attrs().theme, Some(UiTheme::Light));
        assert_eq!(rig.tree.rev(), tree_rev + 1);
        assert_eq!(
            rig.layout.0.lock().unwrap().attrs_rev(),
            attrs_rev,
            "트리 창 쓰기가 레이아웃 창 속성 번호를 올렸다"
        );

        rig.themes
            .set_window_theme(MAIN_WINDOW_LABEL, Some(UiTheme::EInk))
            .unwrap();
        {
            let mgr = rig.layout.0.lock().unwrap();
            assert_eq!(
                mgr.window_attrs(MAIN_WINDOW_LABEL).unwrap().theme,
                Some(UiTheme::EInk)
            );
            assert_eq!(mgr.attrs_rev(), attrs_rev + 1);
            assert_eq!(mgr.version, version, "창 테마는 레이아웃 번호를 안 올린다");
        }
        assert_eq!(rig.tree.attrs().theme, Some(UiTheme::Light));
        assert_eq!(rig.tree.rev(), tree_rev + 1);
    }

    #[test]
    fn an_unknown_window_is_refused_on_read_and_write_but_still_answers_the_boot_pull() {
        let rig = rig("unknown");
        let ghost = "slot-popup-9";
        let attrs_rev = rig.layout.0.lock().unwrap().attrs_rev();

        assert_eq!(
            rig.themes.window_theme(ghost),
            Err(ThemeError::UnknownWindow(ghost.to_string()))
        );
        assert_eq!(
            rig.themes.set_window_theme(ghost, Some(UiTheme::Light)),
            Err(ThemeError::UnknownWindow(ghost.to_string()))
        );
        assert_eq!(rig.layout.0.lock().unwrap().attrs_rev(), attrs_rev);
        assert_eq!(rig.themes.for_window(ghost).theme, DEFAULT_THEME.as_wire());
    }

    #[test]
    fn a_push_sends_each_live_window_its_own_effective_theme() {
        let rig = rig("push");
        rig.settings.set(THEME_DEFAULT, "light").unwrap();
        rig.themes
            .set_window_theme(MAIN_WINDOW_LABEL, Some(UiTheme::EInk))
            .unwrap();
        rig.themes
            .set_window_theme(TREE_WINDOW_ID, Some(UiTheme::Dark))
            .unwrap();
        // 모델에 없는 창(막 만들어지는 팝아웃)은 전역 값을 받는다.
        let windows = Recording::new(&[MAIN_WINDOW_LABEL, TREE_WINDOW_ID, POPUP, "slot-popup-2"]);

        rig.themes.push_effective_themes(&windows).expect("밀기");

        assert_eq!(
            windows.take(),
            sent(&[
                (MAIN_WINDOW_LABEL, "e-ink"),
                (TREE_WINDOW_ID, "dark"),
                (POPUP, "light"),
                ("slot-popup-2", "light"),
            ])
        );
    }

    /// ★`theme.default` 를 바꾸고 다시 밀면 자기 테마가 없는 창만 실제로 바뀐다★ — 밀기는 그때의 설정 값을 읽는다
    /// (캐시하지 않는다).
    #[test]
    fn a_theme_default_change_moves_only_the_windows_without_their_own_theme() {
        let rig = rig("default");
        rig.themes
            .set_window_theme(MAIN_WINDOW_LABEL, Some(UiTheme::EInk))
            .unwrap();
        let windows = Recording::new(&[MAIN_WINDOW_LABEL, TREE_WINDOW_ID, POPUP]);
        rig.themes.push_effective_themes(&windows).expect("밀기");
        assert_eq!(
            windows.take(),
            sent(&[
                (MAIN_WINDOW_LABEL, "e-ink"),
                (TREE_WINDOW_ID, "dark"),
                (POPUP, "dark")
            ])
        );

        rig.settings.set(THEME_DEFAULT, "light").unwrap();
        rig.themes.push_effective_themes(&windows).expect("밀기");

        assert_eq!(
            windows.take(),
            sent(&[
                (MAIN_WINDOW_LABEL, "e-ink"),
                (TREE_WINDOW_ID, "light"),
                (POPUP, "light")
            ])
        );
    }

    /// 부팅 당기기와 밀기가 창마다 같은 값을 낸다 — 한쪽만 다른 출처를 보면 그 창은 부팅과 밀기에서 다른 테마를
    /// 받는다.
    #[test]
    fn a_boot_pull_and_a_push_agree_on_each_window() {
        let rig = rig("agree");
        rig.settings.set(THEME_DEFAULT, "e-ink").unwrap();
        rig.themes
            .set_window_theme(TREE_WINDOW_ID, Some(UiTheme::Light))
            .unwrap();
        let windows = Recording::new(&[MAIN_WINDOW_LABEL, TREE_WINDOW_ID, POPUP]);

        rig.themes.push_effective_themes(&windows).expect("밀기");
        for (label, theme) in windows.take() {
            assert_eq!(rig.themes.for_window(&label).theme, theme, "{label}");
        }
    }

    #[test]
    fn the_control_pushes_after_a_write_and_not_after_a_refused_one() {
        let rig = rig("control");
        let windows = Arc::new(Recording::new(&[MAIN_WINDOW_LABEL, POPUP]));
        let control = ThemeControl::new(Arc::new(rig.themes), windows.clone());

        let written = control.set(POPUP, Some(UiTheme::Light)).unwrap();

        assert_eq!(written.own, Some(UiTheme::Light));
        assert_eq!(
            windows.take(),
            sent(&[(MAIN_WINDOW_LABEL, "dark"), (POPUP, "light")])
        );
        assert!(control.set("slot-popup-9", Some(UiTheme::Light)).is_err());
        assert!(windows.take().is_empty(), "거절된 쓰기 뒤에는 밀지 않는다");
        assert_eq!(control.get(POPUP).unwrap(), written);
    }

    /// ★못 보낸 창이 하나라도 있으면 밀기는 성공이 아니다★ — 명령의 답은 이 값으로 `ThemeControl::set` 이 가른다(ADR-0166 결정 6 개정). 그리고 거기서 멈추지도 않는다.
    #[test]
    fn a_window_that_did_not_receive_it_fails_the_push_after_every_window_was_tried() {
        let rig = rig("refuse");
        let windows = Recording::refusing(
            &[MAIN_WINDOW_LABEL, TREE_WINDOW_ID, POPUP],
            &[TREE_WINDOW_ID],
        );

        let refused = rig
            .themes
            .push_effective_themes(&windows)
            .expect_err("한 창이라도 못 받으면 실패다");

        assert_eq!(
            refused,
            Undelivered::Refused {
                tried: 3,
                failed: vec![FailedWindow {
                    label: TREE_WINDOW_ID.to_string(),
                    reason: CLOSED.to_string(),
                }],
            }
        );
        assert_eq!(windows.take().len(), 3, "실패한 창에서 멈췄다");
    }

    #[test]
    fn a_push_with_no_live_windows_is_a_failure() {
        let mut calls = 0usize;
        let outcome = deliver_per_window(&[], |_label, _payload| {
            calls += 1;
            Ok(())
        });

        assert_eq!(
            outcome,
            Err(Undelivered::NoWindows),
            "빈 명단에 성공으로 답했다"
        );
        assert_eq!(calls, 0);
    }

    /// 값은 못 받은 창 전량을 싣고, 문구는 앞 [`MAX_FAILED_DETAILS`] 창만 사유까지 펼치고 나머지는 수로 뭉친다.
    #[test]
    fn failed_windows_are_counted_in_full_but_described_in_bounded_numbers() {
        let targets: Vec<(String, UiTheme)> = (0..10)
            .map(|n| (format!("slot-popup-{n}"), UiTheme::Dark))
            .collect();

        let refused = deliver_per_window(&targets, |_label, _payload| Err("x".to_string()))
            .expect_err("전부 못 받았다");

        assert_eq!(refused.failed().len(), 10);
        let message = refused.to_string();
        assert!(message.starts_with("10/10 "), "{message}");
        assert_eq!(
            message.matches(": x").count(),
            MAX_FAILED_DETAILS,
            "{message}"
        );
        assert!(message.ends_with(" · 외 7개"), "{message}");
    }

    /// ★그 창이 못 받았으면 성공이 아니다★(ADR-0166 결정 6) — 그래도 쓴 값은 남고, 다른 창까지 다 시도한다.
    #[test]
    fn a_write_the_target_window_did_not_receive_is_an_error_but_stays_written() {
        let rig = rig("target");
        let windows = Arc::new(Recording::refusing(
            &[MAIN_WINDOW_LABEL, TREE_WINDOW_ID, POPUP],
            &[POPUP],
        ));
        let control = ThemeControl::new(Arc::new(rig.themes), windows.clone());

        let refused = control.set(POPUP, Some(UiTheme::Light));

        assert_eq!(
            refused,
            Err(ThemeError::Undelivered {
                window: POPUP.to_string(),
                reason: CLOSED.to_string(),
                others_failed: 0,
            })
        );
        assert_eq!(windows.take().len(), 3, "다른 창에도 시도한다");
        assert_eq!(
            control.get(POPUP),
            Ok(WindowTheme {
                own: Some(UiTheme::Light),
                effective: UiTheme::Light
            }),
            "못 보냈어도 쓴 값은 남는다"
        );
    }

    /// 다른 창만 못 받았으면 그 쓰기는 성공이다 — 그 실패는 밀기가 로그로 남긴다.
    #[test]
    fn a_write_only_other_windows_did_not_receive_is_a_success() {
        let rig = rig("others");
        let windows = Arc::new(Recording::refusing(
            &[MAIN_WINDOW_LABEL, TREE_WINDOW_ID, POPUP],
            &[MAIN_WINDOW_LABEL, TREE_WINDOW_ID],
        ));
        let control = ThemeControl::new(Arc::new(rig.themes), windows.clone());

        let written = control.set(POPUP, Some(UiTheme::EInk));

        assert_eq!(
            written,
            Ok(WindowTheme {
                own: Some(UiTheme::EInk),
                effective: UiTheme::EInk
            })
        );
        assert_eq!(windows.take().len(), 3);
    }

    /// 그 창의 사유는 펼치고 다른 창의 실패는 수로만 붙인다.
    #[test]
    fn an_undelivered_write_names_its_window_and_counts_the_others() {
        let rig = rig("message");
        let windows = Arc::new(Recording::refusing(
            &[MAIN_WINDOW_LABEL, TREE_WINDOW_ID, POPUP],
            &[MAIN_WINDOW_LABEL, TREE_WINDOW_ID, POPUP],
        ));
        let control = ThemeControl::new(Arc::new(rig.themes), windows);

        let message = control
            .set(TREE_WINDOW_ID, Some(UiTheme::Light))
            .expect_err("그 창이 못 받았다")
            .to_string();

        assert!(message.starts_with(TREE_WINDOW_ID), "{message}");
        assert!(message.contains(CLOSED), "{message}");
        assert!(message.ends_with("다른 창 2개도 못 받았다"), "{message}");
    }

    /// 보낼 때는 관문만 쥔다 — 모델 락을 쥔 채 보내면 받는 쪽이 레이아웃을 읽는 순간 교착이다.
    #[test]
    fn a_push_sends_with_the_model_lock_free() {
        struct Probe {
            layout: LayoutState,
            free: Mutex<Vec<bool>>,
        }

        impl ThemeWindows for Probe {
            fn labels(&self) -> Vec<String> {
                vec![MAIN_WINDOW_LABEL.to_string(), TREE_WINDOW_ID.to_string()]
            }

            fn send(&self, _label: &str, _payload: UiSettingsPayload) -> Result<(), String> {
                let free = self.layout.0.try_lock().is_ok();
                self.free.lock().unwrap().push(free);
                Ok(())
            }
        }

        let rig = rig("locks");
        let probe = Probe {
            layout: rig.layout.clone(),
            free: Mutex::default(),
        };

        rig.themes.push_effective_themes(&probe).expect("밀기");

        assert_eq!(*probe.free.lock().unwrap(), [true, true]);
    }
}
