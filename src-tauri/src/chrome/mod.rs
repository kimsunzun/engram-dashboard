//! 창 크롬의 Tauri 어댑터 — [`book::ChromeBook`] 을 잎 락으로 쥐고 OS 창에 입힌다(TRD S26-window-chrome §2-2 · §4-4 ·
//! §5-1). 지금 입히는 것 = OS 창 제목 하나. 웹이 보고한 최소 창 크기는 적기만 한다([`report_min_size`] — OS 최소
//! 크기로 입히기는 아직 없다).
//!
//! 제목 값의 정본은 탭 알림 페이로드(`layout::apply::WindowTabsPayload` 의 `title` · `version`)다. 알림 포트의 Tauri
//! 구현(`commands::layout::TauriEvents`)이 emit 뒤에 [`push_title`] 을 부른다 — 알림은 레이아웃 락 밖에서 여러 스레드가
//! 내 차례가 뒤바뀔 수 있어, 메인 스레드 한 줄 + 장부의 판 비교가 옛 판을 버린다.

mod book;

pub(crate) use book::MinOrder;

use std::sync::Mutex;

use engram_dashboard_base::sync;
use tauri::{AppHandle, Manager};

use crate::layout::WindowTabsPayload;

/// 셸 상태로 등록하는 장부 — 제목 일 · 최소 크기 보고와 `Destroyed` 가 같은 인스턴스를 본다.
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

/// 창 `label` 의 웹이 보고한 최소 창 크기를 적는다 — invoke `report_window_min_width` 가 부른다. 범위 밖이면 `Err` 이고
/// 직전 값을 유지한다(`book::MinSize::new`). 그 OS 창이 없거나 `order` 가 그 창에 받아들인 순번보다 새 것이 아니면
/// 적지 않고 `Ok`(`book::ChromeBook::record_min`). 창 확인과 적기가 한 메인 스레드 일 안이어야 `Destroyed` 의 잊기가 그
/// 사이에 끼지 않는다(`book` 머리) — 비동기가 아닌 Tauri 명령은 메인 스레드에서 돈다(Tauri 2 문서 「Calling Rust from
/// the Frontend」 · 이 셸의 동기 명령이 기대는 사실).
pub(crate) fn report_min_size(
    app: &AppHandle,
    label: &str,
    width: u32,
    height: u32,
    order: MinOrder,
) -> Result<(), String> {
    let state = app
        .try_state::<ChromeState>()
        .ok_or_else(|| "크롬 장부가 등록돼 있지 않다".to_owned())?;
    let open = app.get_webview_window(label).is_some();
    if book::report_min(&state.0, label, width, height, order, open)? {
        tracing::debug!(label, width, height, "최소 창 크기 보고가 바뀌었다");
    }
    // TODO(T-66 U8b): 여기서 판정(`on_min_report` — 보통 상태면 OS 최소 크기를 입히고 최대화면 걷는다 · 열 때 한 번
    //   넓혀 당긴다)을 받아 입힌다(TRD S26-window-chrome §7-7). 지금은 적기만 한다.
    Ok(())
}

/// 창 `label` 의 장부 항목을 잊는다 — 그 OS 창의 `Destroyed`(메인 스레드).
pub(crate) fn forget(app: &AppHandle, label: &str) {
    if let Some(state) = app.try_state::<ChromeState>() {
        sync::lock(&state.0).forget(label);
    }
}
