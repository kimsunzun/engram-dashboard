//! 창 크롬의 Tauri 어댑터 — [`book::ChromeBook`] 을 잎 락으로 쥐고 OS 창에 입힌다(TRD S26-window-chrome §2-2 · §4-4 ·
//! §5-1). 지금 입히는 것 = OS 창 제목 하나.
//!
//! 제목 값의 정본은 탭 알림 페이로드(`layout::apply::WindowTabsPayload` 의 `title` · `version`)다. 알림 포트의 Tauri
//! 구현(`commands::layout::TauriEvents`)이 emit 뒤에 [`push_title`] 을 부른다 — 알림은 레이아웃 락 밖에서 여러 스레드가
//! 내 차례가 뒤바뀔 수 있어, 메인 스레드 한 줄 + 장부의 판 비교가 옛 판을 버린다.

mod book;

use std::sync::Mutex;

use engram_dashboard_base::sync;
use tauri::{AppHandle, Manager};

use crate::layout::WindowTabsPayload;

/// 셸 상태로 등록하는 장부 — 제목 일과 `Destroyed` 가 같은 인스턴스를 본다.
#[derive(Default)]
pub(crate) struct ChromeState(Mutex<book::ChromeBook>);

/// 탭 알림 `tabs` 의 제목을 그 OS 창에 입히는 일을 메인 스레드에 넘기고 기다리지 않는다. 메인 스레드에서 부르면 그
/// 자리에서 돈다(tauri-runtime-wry 2.11.3 `send_user_message`) — 그래서 ★어떤 락도 쥔 채 부르지 않는다★.
pub(crate) fn push_title(app: &AppHandle, tabs: &WindowTabsPayload) {
    let handle = app.clone();
    let label = tabs.label.clone();
    let version = tabs.version;
    let title = tabs.title.clone();
    let queued = app.run_on_main_thread(move || {
        let Some(state) = handle.try_state::<ChromeState>() else {
            tracing::debug!(label, "크롬 장부가 등록돼 있지 않아 제목을 입히지 않는다");
            return;
        };
        book::apply_title(
            &state.0,
            &label,
            version,
            &title,
            handle.get_webview_window(&label),
            |window, text| window.set_title(text).map_err(|e| e.to_string()),
        );
    });
    if let Err(e) = queued {
        tracing::warn!(label = %tabs.label, version, "OS 창 제목 일을 메인 스레드에 넘기지 못했다: {e}");
    }
}

/// 창 `label` 의 장부 항목을 잊는다 — 그 OS 창의 `Destroyed`(메인 스레드).
pub(crate) fn forget(app: &AppHandle, label: &str) {
    if let Some(state) = app.try_state::<ChromeState>() {
        sync::lock(&state.0).forget(label);
    }
}
