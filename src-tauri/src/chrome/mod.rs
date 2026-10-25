//! 창 크롬의 Tauri 어댑터 — [`book::ChromeBook`] 을 잎 락으로 쥐고 OS 창에 입힌다(TRD S26-window-chrome §2-2 · §4-4 ·
//! §5-1 · §7-7). 입히는 것 = OS 창 제목 · 웹이 보고한 최소 창 크기(OS 최소 크기 · 열 때 한 번 넓혀 당기기).
//!
//! 제목 값의 정본은 탭 알림 페이로드(`layout::apply::WindowTabsPayload` 의 `title` · `version`)다. 알림 포트의 Tauri
//! 구현(`commands::layout::TauriEvents`)이 emit 뒤에 [`push_title`] 을 부른다 — 알림은 레이아웃 락 밖에서 여러 스레드가
//! 내 차례가 뒤바뀔 수 있어, 메인 스레드 한 줄 + 장부의 판 비교가 옛 판을 버린다.
//!
//! 최소 크기의 정본은 웹이다(정책 상수 정본 = 화면 — ADR-0227 선례). 보고(invoke `report_window_min_width` →
//! [`report_min_size`]) · 열기 표시([`mark_opening`]) · 크기 바뀜([`on_resized`])이 모두 메인 스레드 일 하나 안에서 장부를
//! 보고 입힌다 — 무엇을 언제 입히나는 `book` 머리.

mod book;

pub(crate) use book::MinOrder;

use std::sync::Mutex;

use engram_dashboard_base::sync;
use tauri::{AppHandle, LogicalSize, Manager, Window};

use crate::layout::WindowTabsPayload;
use crate::state::placement::{self, MonitorArea};

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

/// 셸이 창 `label` 을 막 열었다 — 그 창의 다음 최소 크기 보고가 넓혀 당기기를 하게 열기 표시를 세운다(TRD §7-7 — 창
/// 포트의 새로 짓기 · 맡긴 예열 창의 확정 · 부팅 복원 창 짓기 · 사본 수락의 숨긴 팝아웃 짓기). 창을 지은 뒤에 부른다.
/// 적기는 메인 스레드 일로 넘기고 기다리지 않는다 — 그 일이 창이 있는 것을 본 뒤에만 적는다(`book` 머리). 그 창의 웹
/// 페이지가 보고하기 전에 그 일이 돈다: 메인 스레드는 한 줄이고 보고는 페이지를 읽은 뒤에 같은 줄로 온다. 메인 스레드에서
/// 부르면 그 자리에서 돈다 — ★어떤 락도 쥔 채 부르지 않는다★.
pub(crate) fn mark_opening(app: &AppHandle, label: &str) {
    let handle = app.clone();
    let owned = label.to_owned();
    let queued = app.run_on_main_thread(move || {
        if handle.get_webview_window(&owned).is_none() {
            tracing::debug!(label = owned, "OS 창이 없어 열기 표시를 적지 않는다");
            return;
        }
        if let Some(state) = handle.try_state::<ChromeState>() {
            sync::lock(&state.0).mark_opening(&owned);
        }
    });
    if let Err(e) = queued {
        tracing::warn!(
            label,
            "열기 표시 일을 메인 스레드에 넘기지 못했다(그 창은 넓혀 당기지 않는다): {e}"
        );
    }
}

/// 창 `label` 의 웹이 보고한 최소 창 크기를 적고 입힌다 — invoke `report_window_min_width` 가 부른다. 범위 밖이면 `Err`
/// 이고 직전 값을 유지한다(`book::MinSize::new`). 그 OS 창이 없거나 `order` 가 그 창에 받아들인 순번보다 새 것이 아니면
/// 적지 않고 `Ok`(`book::ChromeBook::record_min`). 보통 크기 창이면 OS 최소 크기를 입히고, 열기 표시가 있으면 먼저 넓혀
/// 당긴다(`book::ChromeBook::on_min_report`). 창 확인 · 상태 읽기 · 적기 · 입히기가 한 메인 스레드 일 안이어야
/// `Destroyed` 의 잊기 · 크기 바뀜이 그 사이에 끼지 않는다(`book` 머리) — 비동기가 아닌 Tauri 명령은 메인 스레드에서
/// 돈다(Tauri 2 문서 「Calling Rust from the Frontend」 · 이 셸의 동기 명령이 기대는 사실).
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
    let window = app
        .get_webview_window(label)
        .map(|window| window.as_ref().window());
    let frame = window.as_ref().map(frame_state);
    let Some(report) = book::report_min(&state.0, label, width, height, order, frame)? else {
        return Ok(());
    };
    if report.verdict == book::MinVerdict::Changed {
        tracing::debug!(label, width, height, "최소 창 크기 보고가 바뀌었다");
    }
    if let Some(window) = window {
        book::apply_min(&state.0, label, report.plan, &window);
    }
    Ok(())
}

/// 창의 크기가 바뀌었다(`WindowEvent::Resized` — 메인 스레드) — 최대화 · 최소화에서 보통 크기로 돌아왔고 적어 둔 최소 크기를
/// 아직 OS 에 입히지 않았으면 입힌다(`book::ChromeBook::on_frame`). ★어떤 락도 쥔 채 부르지 않는다★.
pub(crate) fn on_resized(window: &Window) {
    let Some(state) = window.app_handle().try_state::<ChromeState>() else {
        return;
    };
    let label = window.label();
    let plan = sync::lock(&state.0).on_frame(label, frame_state(window));
    book::apply_min(&state.0, label, plan, window);
}

/// 창 `label` 의 장부 항목을 잊는다 — 그 OS 창의 `Destroyed`(메인 스레드).
pub(crate) fn forget(app: &AppHandle, label: &str) {
    if let Some(state) = app.try_state::<ChromeState>() {
        sync::lock(&state.0).forget(label);
    }
}

// tao 의 최대화 · 최소화 깃발(OS 최소 크기 세터가 건드리는 그 깃발 — `book` 머리)로 가른다. 못 읽으면 건드리지 않는 쪽이다.
fn frame_state(window: &Window) -> book::FrameState {
    match (window.is_maximized(), window.is_minimized()) {
        (Ok(false), Ok(false)) => book::FrameState::Normal,
        _ => book::FrameState::Other,
    }
}

impl book::MinWindow for Window {
    // 보통 자리 = 물리 바깥 위치 · 논리 안쪽 크기(`state::placement` 머리). 작업 영역 = 그 창이 놓인 모니터의 것(숨은 창은
    //   가장 가까운 모니터) · 배율 = 그 창의 배율(논리 폭을 물리로 푸는 값 — tao 의 크기 세터와 같다). 바깥 폭 − 안쪽 폭 =
    //   OS 틀을 걷은 창에도 남는 크기 조절용 보이지 않는 테두리(tao 0.35.3 `calculate_insets_for_dpi`).
    fn widen_to(&self, min_w: u32) -> Result<bool, String> {
        let scale = self.scale_factor().map_err(|e| e.to_string())?;
        let at = self.outer_position().map_err(|e| e.to_string())?;
        let inner = self.inner_size().map_err(|e| e.to_string())?;
        let outer = self.outer_size().map_err(|e| e.to_string())?;
        let bounds = placement::saved_bounds(at.x, at.y, inner.width, inner.height, scale)
            .ok_or_else(|| {
                format!("창 크기를 쓸 수 없는 값으로 읽었다({inner:?} · 배율 {scale})")
            })?;
        let monitor = self
            .current_monitor()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "창이 놓인 모니터를 못 읽었다".to_owned())?;
        let work = monitor.work_area();
        let area = MonitorArea {
            x: work.position.x,
            y: work.position.y,
            w: work.size.width,
            h: work.size.height,
            scale,
        };
        let frame = f64::from(outer.width.saturating_sub(inner.width));
        let fitted = placement::fit_min_width(bounds, f64::from(min_w), area, frame);
        if fitted == bounds {
            return Ok(false);
        }
        // QA 재료(AC27 · AC34 — 넓힌 폭 · 당긴 자리를 작업 영역과 견준다).
        tracing::debug!(
            module = "chrome",
            label = self.label(),
            from = ?bounds,
            to = ?fitted,
            work = ?area,
            frame,
            "넓혀 당기기"
        );
        // 위치를 바꿀 때만 위치를 입힌다 — 크기만 넓히면 왼쪽 끝이 그대로다(크기 세터는 위치를 두고 크기만 바꾼다).
        let pos = (fitted.x() != bounds.x())
            .then(|| (fitted.x().round() as i32, fitted.y().round() as i32));
        placement::move_steps(self, pos, Some((fitted.w(), fitted.h())))
            .map_err(|failed| format!("{:?}: {}", failed.step, failed.error))?;
        Ok(true)
    }

    fn apply_min_size(&self, min: book::MinSize) -> Result<(), String> {
        self.set_min_size(Some(LogicalSize::new(
            f64::from(min.width),
            f64::from(min.height),
        )))
        .map_err(|e| e.to_string())
    }
}
