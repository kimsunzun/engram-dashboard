//! 창 자리 — `Moved` · `Resized` 를 모델에 적고([`record`]), 저장된 자리를 창에 입힌다([`restore_windows`] — 부팅
//! 단계 ⑧ ⑨ · TRD S21-storage §6-3 · §6-5).
//!
//! - ★위치 = 물리 바깥 위치(`outer_position` 그대로), 크기 = 창의 배율로 나눈 논리 안쪽 크기★. 논리 위치는 배율이
//!   다른 모니터가 섞이면 한 값이 두 모니터를 가리켜(100% 오른쪽에 150% — 논리 x 1280..1920 띠) 창이 다른
//!   모니터로 복원된다 — ★위치를 논리로 되돌리지 말 것★. 위치를 물리로 적는 것은 tauri-plugin-window-state 와
//!   같다(그 플러그인은 크기도 물리로 적는다). 크기를 논리로 두면 배율 설정이 바뀌어도 보이는 크기가 지켜지고, 팝아웃
//!   빌더의 크기 인자(논리)에 그대로 들어간다.
//! - 창이 실제로 놓이는 모니터는 물리 위치가 정한다 — [`locate`] 가 고르는 모니터는 버릴지 판정과 팝아웃 빌더의 첫
//!   자리에만 쓴다.
//! - ★창 게터 · 세터는 어느 락보다 먼저(또는 놓은 뒤) 부르고 락 안에서는 값만 적는다★ — 레이아웃 락 보유 중 OS
//!   호출 금지(§6-3 · `layout/apply.rs` 머리 「락 규율」).
//! - 패닉하지 않는다 — 창 사건 처리기에서 불린다(릴리스는 `panic = "abort"`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, PhysicalPosition, Window};

use super::convert::TREE_WINDOW_ID;
use super::tree_attrs::TreeAttrs;
use crate::layout::{LayoutState, WindowAttrs, WindowBounds, WindowPlacement, MAIN_WINDOW_LABEL};
use crate::ui_settings::EffectiveThemes;

/// 모니터 하나 — OS 가 알려 준 물리 픽셀 사각형과 그 배율.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonitorArea {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub scale: f64,
}

/// [`locate`] 가 고른 자리 — 물리 바깥 위치와 고른 모니터의 배율.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Landing {
    pub x: i32,
    pub y: i32,
    pub scale: f64,
}

/// 저장된 자리가 갈 모니터. 모니터마다 「창이 그 모니터에 있다면」의 물리 사각형(저장된 위치 그대로 · 논리 크기 ×
/// 그 모니터의 배율)을 그 모니터와 겹쳐 가장 넓게 걸치는 것을 고른다(같으면 먼저 열거된 것). `None` = 어느 모니터에도
/// 걸치지 않는다(넓이 0 인 맞닿음 포함) — 그 자리는 버리고 기본 자리로 연다(§6-5 ⑧ ⑨ · 사용자 결정 F8). 배율이
/// 유한한 양수가 아닌 모니터는 없는 것으로 친다.
pub fn locate(bounds: WindowBounds, monitors: &[MonitorArea]) -> Option<Landing> {
    let (left, top) = (bounds.x(), bounds.y());
    let mut best: Option<(f64, f64)> = None;
    for m in monitors {
        if !(m.scale.is_finite() && m.scale > 0.0) {
            continue;
        }
        let (mx, my) = (f64::from(m.x), f64::from(m.y));
        let (mr, mb) = (mx + f64::from(m.w), my + f64::from(m.h));
        let w = (left + bounds.w() * m.scale).min(mr) - left.max(mx);
        let h = (top + bounds.h() * m.scale).min(mb) - top.max(my);
        if w > 0.0 && h > 0.0 {
            let area = w * h;
            if best.is_none_or(|(widest, _)| area > widest) {
                best = Some((area, m.scale));
            }
        }
    }
    best.map(|(_, scale)| Landing {
        x: to_px(left),
        y: to_px(top),
        scale,
    })
}

fn to_px(v: f64) -> i32 {
    v.round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

/// 게터 값(물리 바깥 위치 · 물리 안쪽 크기 · 창의 배율)을 적을 자리로 — 위치는 그대로, 크기는 배율로 나눈다(모듈
/// 머리). 배율이 유한한 양수가 아니거나 [`WindowBounds::new`] 가 거절하면(크기 0 — 최소화 등) `None`.
pub fn saved_bounds(x: i32, y: i32, w: u32, h: u32, scale: f64) -> Option<WindowBounds> {
    if !(scale.is_finite() && scale > 0.0) {
        return None;
    }
    WindowBounds::new(
        f64::from(x),
        f64::from(y),
        f64::from(w) / scale,
        f64::from(h) / scale,
    )
}

/// 부팅 단계 ⑨ — 모델의 팝아웃마다 `open` 으로 창을 연다(label 발급 순). 못 연 창은 모델에서 지운다 — 다음 주기
/// 저장이 싣는다. 돌려주는 값 = 지운 label. ★`open` 은 락 밖에서 부른다★(창 만들기 = OS 호출).
///
/// ★부팅 전용이다★ — 모델의 팝아웃이 아직 창이 없다는 것이 전제다. 런타임에 부르면 떠 있는 팝아웃마다 이미 있는
/// label 로 창 만들기가 실패해, 그 팝아웃을 모델에서 지우고 OS 창은 그대로 남는다. 런타임 복원(P3c1)은 다른 길을 쓴다.
pub fn open_restored_popouts(
    layout: &LayoutState,
    mut open: impl FnMut(&str, WindowAttrs) -> Result<(), String>,
) -> Vec<String> {
    let mut popouts: Vec<(String, WindowAttrs)> = match layout.0.lock() {
        Ok(mgr) => mgr
            .windows
            .iter()
            .filter(|(label, _)| label.as_str() != MAIN_WINDOW_LABEL)
            .map(|(label, window)| (label.clone(), window.attrs))
            .collect(),
        Err(_) => {
            tracing::error!(
                module = "state",
                "레이아웃 락에 독이 들어 복원한 팝아웃 창을 열지 못했다"
            );
            return Vec::new();
        }
    };
    // 같은 접두 + 십진 순번이라 (길이, 글자) 순이 발급 순이다 — 저장된 순서대로 쌓인다.
    popouts.sort_by_key(|(label, _)| (label.len(), label.clone()));

    let failed: Vec<String> = popouts
        .into_iter()
        .filter_map(|(label, attrs)| match open(&label, attrs) {
            Ok(()) => None,
            Err(e) => {
                tracing::warn!(
                    module = "state",
                    label,
                    error = %e,
                    "복원한 팝아웃 창을 못 열어 모델에서 지운다"
                );
                Some(label)
            }
        })
        .collect();
    if !failed.is_empty() {
        match layout.0.lock() {
            Ok(mut mgr) => {
                for label in &failed {
                    let _ = mgr.close_window(label);
                }
            }
            Err(_) => tracing::error!(
                module = "state",
                ?failed,
                "레이아웃 락에 독이 들어 못 연 팝아웃을 모델에서 지우지 못했다"
            ),
        }
    }
    failed
}

/// main 의 저장된 최대화를 main 이 처음 보일 때로 미뤘다는 표식 — 세우는 쪽 = [`restore_windows`](그때 main 이
/// 숨어 있으면) · 거두는 쪽 = [`apply_deferred_maximize`](보이기 경로 `tray::actions::show_main_ui`).
///
/// - 숨은 창을 최대화하면 tao 가 `SW_MAXIMIZE`(창을 보이고 활성화한다)를 낸 뒤에야 `SW_HIDE` 로 되숨긴다(tao 0.35.3
///   `platform_impl/windows/window_state.rs` `apply_diff`) — `--hidden` 부팅에 main 이 번쩍이고 포커스를 가져간다.
/// - 서 있는 동안은 main 의 자리를 적지 않는다([`record`]) — 숨은 동안 오는 `Moved` · `Resized` 는 복원이 입힌 보통
///   자리를 「최대화 아님」으로 읽어, 적으면 모델의 최대화 표식이 내려간다(보이기 전에 끝내면 다음 부팅이 최대화를
///   잃는다).
/// - 지금은 main 이 설정대로 보인 채 만들어지고 숨기기는 setup 끝이라(`lib.rs`) 복원 때 숨어 있지 않아 이 길을 타지
///   않는다 — main 을 숨긴 채 만들 때(TRD S21-storage §4 「덤」) 탄다.
#[derive(Debug, Default)]
pub struct DeferredMaximize(AtomicBool);

impl DeferredMaximize {
    fn defer(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    fn is_pending(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    /// 서 있었으면 내리고 `true` — 미룬 최대화는 한 번만 입힌다.
    fn take(&self) -> bool {
        self.0.swap(false, Ordering::SeqCst)
    }
}

// ── Tauri 쪽 — 창이 있어야 돌아 GUI 실측 몫이다 ──────────────────────────────

/// 부팅 단계 ⑧ ⑨ — main · 트리 창에 저장된 자리를 입히고(숨은 main 의 최대화는 보일 때로 미룬다 —
/// [`DeferredMaximize`]), 복원한 팝아웃 창을 열고(못 열면 모델에서 지운다), 모든 창의 유효 테마를 한 번
/// 민다(§5-6). ★부르는 쪽은 아무 락도 쥐지 않는다★. 뒤따르는 구독 재계산(⑩)은 부르는 쪽 몫이다 — 이 함수가 끝난
/// 모델이 마지막 창 묶음이다.
///
/// ★부팅 전용이다★ — 팝아웃을 [`open_restored_popouts`] 로 연다(그 전제 · 런타임에 부르면 생기는 일은 거기).
pub fn restore_windows(
    app: &AppHandle,
    layout: &LayoutState,
    tree: &TreeAttrs,
    themes: &EffectiveThemes,
) {
    let monitors = monitors(app);
    // 두 칸은 락마다 따로 짧게 읽는다(겹쳐 잡지 않는다 — §6-3).
    let main = match layout.0.lock() {
        Ok(mgr) => mgr.window_attrs(MAIN_WINDOW_LABEL).ok(),
        Err(_) => {
            tracing::debug!(
                module = "state",
                "레이아웃 락에 독이 들어 main 창 자리를 입히지 않는다"
            );
            None
        }
    };
    match (main, app.get_webview_window(MAIN_WINDOW_LABEL)) {
        (Some(attrs), Some(window)) => {
            let window = window.as_ref().window();
            place_saved(&window, attrs.bounds, &monitors);
            if attrs.maximized {
                maximize_main(app, &window);
            }
        }
        (Some(_), None) => tracing::debug!(
            module = "state",
            "main 창이 없어 저장된 자리를 입히지 않는다"
        ),
        (None, _) => {}
    }
    // 트리 창은 자리만 입힌다 — 최대화를 싣지 않는다(`state::tree_attrs` 머리).
    if let Some(window) = app.get_webview_window(TREE_WINDOW_ID) {
        place_saved(&window.as_ref().window(), tree.attrs().bounds, &monitors);
    }
    open_restored_popouts(layout, |label, attrs| {
        open_popout(app, label, attrs, &monitors)
    });
    crate::commands::settings::push_themes(app, themes);
}

/// 복원한 팝아웃 창 하나를 연다 — 저장된 자리(어느 모니터에도 안 걸치면 label 의 기본 자리) · 최대화 · 첫 자리
/// 기록. 창은 모델에 이미 있어야 한다(기록이 그 항목에 적는다).
fn open_popout(
    app: &AppHandle,
    label: &str,
    attrs: WindowAttrs,
    monitors: &[MonitorArea],
) -> Result<(), String> {
    let saved = attrs
        .bounds
        .and_then(|bounds| land(label, bounds, monitors).map(|at| (bounds, at)));
    let first = saved.map(|(bounds, at)| {
        (
            LogicalPosition::new(f64::from(at.x) / at.scale, f64::from(at.y) / at.scale),
            LogicalSize::new(bounds.w(), bounds.h()),
        )
    });
    let window = crate::commands::popout::build_runtime_window(app, label, first)?;
    let window = window.as_ref().window();
    if let Some((bounds, at)) = saved {
        place(&window, bounds, at);
    }
    if attrs.maximized {
        maximize(&window);
    }
    record(&window);
    Ok(())
}

/// 이미 있는 창에 저장된 보통 자리를 입힌다. 없거나 어느 모니터에도 안 걸치는 자리면 창은 지금 자리에 남는다.
fn place_saved(window: &Window, bounds: Option<WindowBounds>, monitors: &[MonitorArea]) {
    if let Some(bounds) = bounds {
        if let Some(at) = land(window.label(), bounds, monitors) {
            place(window, bounds, at);
        }
    }
}

fn land(label: &str, bounds: WindowBounds, monitors: &[MonitorArea]) -> Option<Landing> {
    let at = locate(bounds, monitors);
    if at.is_none() {
        tracing::info!(
            module = "state",
            label,
            "저장된 창 자리가 어느 모니터에도 안 걸쳐 버렸다 — 기본 자리로 연다"
        );
    }
    at
}

fn place(window: &Window, bounds: WindowBounds, at: Landing) {
    let label = window.label();
    // 위치를 먼저, 크기를 뒤에 — tao 는 논리 크기를 창이 그때 가진 배율로 풀고(`set_inner_size`) 그 배율은
    //   WM_DPICHANGED 처리에서 갈린다. 크기를 뒤에 놓아야 도착한 모니터의 배율로 풀린다. tauri-plugin-window-state
    //   도 크기를 앞에 놓던 판이 배율 다른 모니터에서 크기가 튀어 이 순서로 바꿨다(plugins-workspace #2583).
    //   [미검증 — WM_DPICHANGED 가 위치 세터 안에서 처리되는지(숨은 트리 창에도 오는지 포함). 늦게 오면 tao 가
    //   그때 크기를 다시 재고, Windows 11 에서는 OS 가 제안한 사각형을 그대로 입혀 저장된 위치 · 크기에서 몇 px
    //   어긋날 수 있다 — GUI 실측 몫]
    if let Err(e) = window.set_position(PhysicalPosition::new(at.x, at.y)) {
        tracing::warn!(module = "state", label, error = %e, "저장된 창 위치를 못 입혔다");
    }
    if let Err(e) = window.set_size(LogicalSize::new(bounds.w(), bounds.h())) {
        tracing::warn!(module = "state", label, error = %e, "저장된 창 크기를 못 입혔다");
    }
}

// 보통 자리를 입힌 뒤에 부른다 — tao 의 위치 세터는 최대화 표식을 내린다(F8).
fn maximize(window: &Window) {
    if let Err(e) = window.maximize() {
        tracing::warn!(module = "state", label = window.label(), error = %e, "저장된 최대화를 못 입혔다");
    }
}

// 숨은 main 은 보일 때로 미룬다([`DeferredMaximize`]). 보임 여부를 못 읽으면 지금 입힌다 — 보이는 창에 미루면
//   보이기 경로가 다시 불릴 때까지 최대화가 안 된다.
fn maximize_main(app: &AppHandle, window: &Window) {
    let hidden = matches!(window.is_visible(), Ok(false));
    match app.try_state::<DeferredMaximize>() {
        Some(deferred) if hidden => deferred.defer(),
        _ => maximize(window),
    }
}

/// 보이기 경로가 main 을 보인 직후 부른다 — 미룬 최대화가 있으면 입힌다([`DeferredMaximize`]).
pub fn apply_deferred_maximize(window: &Window) {
    if window
        .app_handle()
        .try_state::<DeferredMaximize>()
        .is_some_and(|deferred| deferred.take())
    {
        maximize(window);
    }
}

fn monitors(app: &AppHandle) -> Vec<MonitorArea> {
    match app.available_monitors() {
        Ok(list) => list
            .iter()
            .map(|m| MonitorArea {
                x: m.position().x,
                y: m.position().y,
                w: m.size().width,
                h: m.size().height,
                scale: m.scale_factor(),
            })
            .collect(),
        Err(e) => {
            tracing::warn!(
                module = "state",
                error = %e,
                "모니터 목록을 못 읽어 저장된 창 자리를 모두 버린다"
            );
            Vec::new()
        }
    }
}

/// 창 게터 한 벌. 최소화 · 최대화 여부를 못 읽으면 `None`(적지 않는다 — 최소화 중의 자리를 보통 자리로 적게 된다).
/// ★어느 락도 쥐지 않고 부른다★.
fn read(window: &Window) -> Option<WindowPlacement> {
    let label = window.label();
    let flags = window
        .is_minimized()
        .and_then(|minimized| Ok((minimized, window.is_maximized()?)));
    let (minimized, maximized) = match flags {
        Ok(flags) => flags,
        Err(e) => {
            tracing::debug!(
                module = "state",
                label,
                error = %e,
                "창의 최소화 · 최대화 여부를 못 읽어 이번 자리는 적지 않는다"
            );
            return None;
        }
    };
    let bounds = (|| {
        let scale = window.scale_factor()?;
        let at = window.outer_position()?;
        let size = window.inner_size()?;
        Ok::<_, tauri::Error>(saved_bounds(at.x, at.y, size.width, size.height, scale))
    })()
    .unwrap_or_else(|e| {
        tracing::debug!(
            module = "state",
            label,
            error = %e,
            "창 위치 · 크기를 못 읽어 이번 자리는 적지 않는다"
        );
        None
    });
    Some(WindowPlacement {
        minimized,
        maximized,
        bounds,
    })
}

/// 창 하나의 지금 자리를 그 창의 칸에 적는다 — 트리 창은 트리 칸, 그 밖은 레이아웃 모델의 그 label 항목. 모델에
/// 없는 label(닫힌 뒤 늦게 온 사건 · 모델에 들기 전의 새 창)은 버린다 — 팝아웃 label 은 재사용되지 않아 다른 창에
/// 잘못 적히지 않는다(`PopupCounter`). 미룬 최대화가 서 있는 동안의 main 도 적지 않는다([`DeferredMaximize`]).
/// 규칙(최소화 · 최대화)은 [`WindowAttrs`] 의 `observe`.
pub fn record(window: &Window) {
    let label = window.label();
    let app = window.app_handle();
    if label == MAIN_WINDOW_LABEL
        && app
            .try_state::<DeferredMaximize>()
            .is_some_and(|deferred| deferred.is_pending())
    {
        tracing::debug!(
            module = "state",
            label,
            "main 의 최대화를 보일 때로 미뤄 둔 동안이라 이번 자리는 적지 않는다"
        );
        return;
    }
    let Some(seen) = read(window) else {
        return;
    };
    if label == TREE_WINDOW_ID {
        match app.try_state::<Arc<TreeAttrs>>() {
            Some(tree) => tree.observe_placement(seen),
            None => tracing::debug!(
                module = "state",
                label,
                "트리 창 칸이 등록돼 있지 않아 창 자리를 적지 않는다"
            ),
        }
        return;
    }
    let Some(layout) = app.try_state::<LayoutState>() else {
        tracing::debug!(
            module = "state",
            label,
            "레이아웃 상태가 등록돼 있지 않아 창 자리를 적지 않는다"
        );
        return;
    };
    // 창을 끌어 옮기는 동안 사건마다 불린다 — 독 든 락 로그는 debug 로 둔다(그 락의 다른 쓰는 쪽이 이미 크게 남긴다).
    let Ok(mut mgr) = layout.0.lock() else {
        tracing::debug!(
            module = "state",
            label,
            "레이아웃 락에 독이 들어 창 자리를 적지 않는다"
        );
        return;
    };
    if let Err(e) = mgr.observe_window_placement(label, seen) {
        tracing::debug!(module = "state", label, error = %e, "모델에 없는 창의 자리라 버린다");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::ViewManager;

    fn monitor(x: i32, y: i32, w: u32, h: u32, scale: f64) -> MonitorArea {
        MonitorArea { x, y, w, h, scale }
    }

    fn rect(x: f64, y: f64, w: f64, h: f64) -> WindowBounds {
        WindowBounds::new(x, y, w, h).unwrap()
    }

    fn landing(x: i32, y: i32, scale: f64) -> Option<Landing> {
        Some(Landing { x, y, scale })
    }

    const PRIMARY: MonitorArea = MonitorArea {
        x: 0,
        y: 0,
        w: 1920,
        h: 1080,
        scale: 1.0,
    };

    /// PRIMARY 오른쪽의 150% 모니터 — 논리 x 로는 1280 부터라 PRIMARY 의 [1280, 1920) 과 겹친다.
    const RIGHT: MonitorArea = MonitorArea {
        x: 1920,
        y: 0,
        w: 2560,
        h: 1440,
        scale: 1.5,
    };

    // ── 모니터 고르기 ──

    #[test]
    fn a_rect_on_a_monitor_keeps_its_physical_position() {
        assert_eq!(
            locate(rect(80.0, 60.0, 1280.0, 800.0), &[PRIMARY]),
            landing(80, 60, 1.0)
        );
        // 일부만 걸쳐도 걸친 것이다.
        assert_eq!(
            locate(rect(-1000.0, 100.0, 1280.0, 800.0), &[PRIMARY]),
            landing(-1000, 100, 1.0)
        );
    }

    #[test]
    fn a_rect_off_every_monitor_is_discarded() {
        let both = [PRIMARY, RIGHT];
        assert_eq!(locate(rect(-3000.0, 100.0, 800.0, 600.0), &both), None);
        assert_eq!(
            locate(rect(100.0, 1500.0, 800.0, 600.0), &both),
            None,
            "아래"
        );
        // 변만 맞닿으면 넓이 0 — 걸친 것이 아니다.
        assert_eq!(locate(rect(4480.0, 0.0, 800.0, 600.0), &both), None);
        assert_eq!(locate(rect(-800.0, 0.0, 800.0, 600.0), &both), None);
        assert_eq!(
            locate(rect(0.0, 0.0, 800.0, 600.0), &[]),
            None,
            "모니터 없음"
        );
    }

    #[test]
    fn the_rect_on_each_monitor_is_its_logical_size_times_that_monitors_scale() {
        // 논리 크기 그대로면 1800..1900 이라 RIGHT 에 안 닿고, RIGHT 의 배율로 키우면 1800..1950 이라 걸친다.
        assert_eq!(
            locate(rect(1800.0, 100.0, 100.0, 300.0), &[RIGHT]),
            landing(1800, 100, 1.5)
        );
        // 넓이도 키운 사각형으로 잰다 — PRIMARY = 220 × 300 · RIGHT = 380 × 450(키우지 않으면 180 × 300 이라 PRIMARY
        //   가 이긴다). 위치는 어느 쪽이든 저장된 물리 위치 그대로다.
        assert_eq!(
            locate(rect(1700.0, 100.0, 400.0, 300.0), &[PRIMARY, RIGHT]),
            landing(1700, 100, 1.5)
        );
    }

    #[test]
    fn monitors_with_an_unusable_scale_are_ignored() {
        let rect = rect(80.0, 60.0, 400.0, 300.0);
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(locate(rect, &[monitor(0, 0, 1920, 1080, scale)]), None);
        }
    }

    #[test]
    fn negative_coordinates_and_fractional_positions() {
        let left = monitor(-2560, 0, 2560, 1440, 1.5);
        let above = monitor(0, -1440, 2560, 1440, 1.25);
        let all = [PRIMARY, left, above];
        assert_eq!(
            locate(rect(-2000.0, 200.0, 800.0, 600.0), &all),
            landing(-2000, 200, 1.5)
        );
        assert_eq!(
            locate(rect(100.0, -1000.0, 800.0, 600.0), &all),
            landing(100, -1000, 1.25)
        );
        // 손으로 고친 파일의 소수 위치는 반올림한다(반은 0 에서 먼 쪽).
        assert_eq!(
            locate(rect(10.6, -10.5, 800.0, 600.0), &[PRIMARY]),
            landing(11, -11, 1.0)
        );
    }

    // ── 게터 값 → 적을 자리 ──

    #[test]
    fn the_position_stays_physical_and_the_size_is_divided_by_the_window_scale() {
        assert_eq!(
            saved_bounds(3000, 150, 1200, 900, 1.5),
            Some(rect(3000.0, 150.0, 800.0, 600.0))
        );
        assert_eq!(
            saved_bounds(-7, 0, 1001, 800, 1.25),
            Some(rect(-7.0, 0.0, 800.8, 640.0))
        );
    }

    #[test]
    fn a_zero_size_or_an_unusable_scale_gives_no_bounds() {
        assert_eq!(saved_bounds(-32000, -32000, 0, 0, 1.0), None, "최소화");
        for scale in [0.0, -2.0, f64::NAN] {
            assert_eq!(saved_bounds(0, 0, 100, 100, scale), None);
        }
    }

    // ── 적은 자리가 같은 모니터로 돌아온다 ──

    #[test]
    fn a_window_entirely_on_either_monitor_lands_back_on_that_monitor() {
        // (물리 바깥 위치 · 물리 안쪽 크기 · 그 모니터) — 둘째 · 넷째는 논리 위치였다면 [1280, 1920) 띠에 들어 두
        //   모니터를 다 가리켰을 자리다.
        let cases = [
            (1000, 100, 800, 600, PRIMARY),
            (1300, 100, 400, 400, PRIMARY),
            (3000, 300, 1200, 900, RIGHT),
            (2100, 300, 1200, 900, RIGHT),
            (1920, 0, 600, 450, RIGHT),
        ];
        for (x, y, w, h, on) in cases {
            let saved = saved_bounds(x, y, w, h, on.scale).unwrap();
            for order in [[PRIMARY, RIGHT], [RIGHT, PRIMARY]] {
                assert_eq!(
                    locate(saved, &order),
                    landing(x, y, on.scale),
                    "({x}, {y}) {w}x{h} · {order:?}"
                );
            }
            let size = LogicalSize::new(saved.w(), saved.h()).to_physical::<u32>(on.scale);
            assert_eq!(
                (size.width, size.height),
                (w, h),
                "크기도 같은 물리 px 로 돌아온다"
            );
        }
    }

    // ── ⑨ 복원한 팝아웃 열기 ──

    fn layout_with_popouts(labels: &[&str]) -> LayoutState {
        let mut mgr = ViewManager::new();
        for label in labels {
            mgr.create_window(label).unwrap();
        }
        LayoutState(Arc::new(std::sync::Mutex::new(mgr)))
    }

    #[test]
    fn popouts_open_in_issue_order_with_their_attributes() {
        let layout = layout_with_popouts(&["slot-popup-10", "slot-popup-2"]);
        let normal = rect(-1920.0, 40.0, 720.0, 500.0);
        {
            let mut mgr = layout.0.lock().unwrap();
            for (maximized, at) in [(false, normal), (true, rect(-1928.0, -8.0, 1920.0, 1040.0))] {
                let seen = WindowPlacement {
                    minimized: false,
                    maximized,
                    bounds: Some(at),
                };
                mgr.observe_window_placement("slot-popup-2", seen).unwrap();
            }
        }

        let mut opened = Vec::new();
        let removed = open_restored_popouts(&layout, |label, attrs| {
            assert!(layout.0.try_lock().is_ok(), "락 밖에서 연다");
            opened.push((label.to_string(), attrs));
            Ok(())
        });

        assert!(removed.is_empty());
        let labels: Vec<&str> = opened.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, ["slot-popup-2", "slot-popup-10"]);
        assert_eq!(opened[0].1.bounds, Some(normal));
        assert!(opened[0].1.maximized);
        assert_eq!(opened[1].1, WindowAttrs::default());
    }

    #[test]
    fn a_popout_that_fails_to_open_is_removed_from_the_model() {
        let layout = layout_with_popouts(&["slot-popup-1", "slot-popup-2"]);
        let version = layout.0.lock().unwrap().version;

        let removed = open_restored_popouts(&layout, |label, _| {
            if label == "slot-popup-1" {
                Err("창 생성 실패(시험)".into())
            } else {
                Ok(())
            }
        });

        assert_eq!(removed, ["slot-popup-1"]);
        let mgr = layout.0.lock().unwrap();
        let mut windows = mgr.list_windows();
        windows.sort();
        assert_eq!(windows, [MAIN_WINDOW_LABEL, "slot-popup-2"]);
        assert!(
            mgr.version > version,
            "지운 것이 레이아웃 변경이라 기록기가 싣는다"
        );
    }

    #[test]
    fn a_model_without_popouts_opens_nothing() {
        let layout = LayoutState::new();
        let removed = open_restored_popouts(&layout, |label, _| {
            panic!("열 팝아웃이 없다: {label}");
        });
        assert!(removed.is_empty());
    }

    // ── 미룬 최대화 ──

    #[test]
    fn a_deferred_maximize_is_applied_once() {
        let deferred = DeferredMaximize::default();
        assert!(!deferred.is_pending());
        assert!(!deferred.take(), "미룬 것이 없으면 입히지 않는다");

        deferred.defer();
        assert!(
            deferred.is_pending(),
            "서 있는 동안 main 의 자리를 적지 않는다"
        );
        assert!(deferred.take());
        assert!(!deferred.is_pending());
        assert!(!deferred.take(), "다음 보이기는 다시 최대화하지 않는다");
    }
}
