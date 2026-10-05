//! 창 자리 — `Moved` · `Resized` 를 모델에 적고([`record`]), 저장된 자리를 창에 입힌다([`restore_windows`] — 부팅
//! 단계 ⑧ ⑨ · TRD S21-storage §6-3 · §6-5 · 런타임 복원 수락의 창 포트 [`TauriRestoreWindows`] — §6-7 ② ④).
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

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, PhysicalPosition, Window};

use super::convert::TREE_WINDOW_ID;
use super::restore::RestoreWindows;
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

/// 저장된 최대화를 그 창이 처음 보일 때로 미룬 창의 label 들.
///
/// - 세우는 쪽 = main 에 자리 · 최대화를 입히는 길(부팅 ⑧ [`restore_windows`] · 런타임 복원 수락
///   [`TauriRestoreWindows`] — main 이 숨어 있으면 보일 때까지, 보이면 그 자리에서 입혀질 때까지 —
///   [`DeferredMaximize::apply_now`]) · 런타임 복원 수락이 숨긴 채 만든 최대화 팝아웃
///   (`RestoreWindows::open_hidden`). 거두는 쪽 = 입히기에 성공한 그 길 · [`apply_deferred_maximize`](보이기 경로 —
///   `tray::actions::show_main_ui` · 복원 수락의 `set_shown`) · 최대화가 아닌 사본을 입히는 런타임 복원(main) ·
///   창 소멸([`forget_deferred_maximize`]).
/// - 숨은 창을 최대화하면 tao 가 `SW_MAXIMIZE`(창을 보이고 활성화한다)를 낸 뒤에야 `SW_HIDE` 로 되숨긴다(tao 0.35.3
///   `platform_impl/windows/window_state.rs` `apply_diff`) — `--hidden` 부팅에 main 이 번쩍이고 포커스를 가져간다.
/// - 서 있는 동안은 그 창의 자리를 적지 않는다([`record`]) — 숨은 동안 오는 `Moved` · `Resized` 는 복원이 입힌 보통
///   자리를 「최대화 아님」으로 읽어, 적으면 모델의 최대화 표식이 내려간다(보이기 전에 끝내면 다음 부팅이 최대화를
///   잃는다).
/// - 부팅에서는 main 이 설정대로 보인 채 만들어지고 숨기기는 setup 끝이라(`lib.rs`) 숨어 있지 않아 최대화를 못
///   입혔을 때만 남는다 — main 을 숨긴 채 만들 때(TRD S21-storage §4 「덤」) 보일 때까지 남는다. 런타임 복원 수락은
///   숨은 main(트레이 숨기기 · `--hidden` 부팅 뒤 LLM 의 답)에 보일 때까지 남는다.
/// - 락은 잎이다 — 쥔 채 창 호출 · 로그 · 다른 락을 하지 않는다.
#[derive(Debug, Default)]
pub struct DeferredMaximize(Mutex<BTreeSet<String>>);

impl DeferredMaximize {
    fn labels(&self) -> MutexGuard<'_, BTreeSet<String>> {
        // 칸이 집합 하나라 반쯤 바뀐 상태가 없다 — 창 사건 처리기에서 불려 패닉하면 안 된다(모듈 머리).
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn defer(&self, label: &str) {
        self.labels().insert(label.to_string());
    }

    fn is_pending(&self, label: &str) -> bool {
        self.labels().contains(label)
    }

    /// 서 있었으면 내리고 `true` — 미룬 최대화는 한 번만 입힌다. 창 소멸의 거두기도 이것이다.
    fn take(&self, label: &str) -> bool {
        self.labels().remove(label)
    }

    /// 서 있으면 `maximize` 로 입히고, 입혀졌을 때만 내린다 — `true` = 이번에 입히고 내렸다(부르는 쪽이 자리를
    /// 적는다). ★못 입혔으면 남긴다★ — 내리면 보통 자리를 「최대화 아님」으로 적어 모델의 최대화가 지워지고 다시
    /// 입힐 길도 없다. 남기면 다음 보이기가 다시 입힌다. `maximize` 는 락 밖에서 부른다.
    fn apply(&self, label: &str, maximize: impl FnOnce() -> bool) -> bool {
        self.is_pending(label) && maximize() && self.take(label)
    }

    /// 보이는 창에 지금 입힌다 — 세운 채 `place`(보통 자리)와 `maximize` 를 부르고, 입혀졌을 때만 내린다. `true` =
    /// 입혔다. ★못 입혔으면 남긴다★ — [`record`] 가 그 창을 「최대화 아님」으로 적지 않고(런타임 복원 수락 뒤의 자리
    /// 기록 포함) 다음 보이기가 다시 입힌다([`Self::apply`] 와 같은 까닭). `place` · `maximize` 는 락 밖에서 부른다.
    /// - ★`place` 보다 먼저 세운다★ — 자리 입히기가 낸 `Moved` · `Resized` 가 보통 자리를 「최대화 아님」으로 적지
    ///   않게.
    /// - ★사이에 보이기가 미룸을 입히고 거뒀어도 `maximize` 를 부른다★ — 그 뒤의 위치 세터가 최대화를 내렸다(F8).
    ///   못 입혔으면 그래서 다시 세운다.
    fn apply_now(
        &self,
        label: &str,
        place: impl FnOnce(),
        maximize: impl FnOnce() -> bool,
    ) -> bool {
        self.defer(label);
        place();
        let applied = maximize();
        if applied {
            self.take(label);
        } else {
            self.defer(label);
        }
        applied
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
            let at = attrs.bounds.and_then(|bounds| {
                land(MAIN_WINDOW_LABEL, bounds, &monitors, Fallback::StayPut).map(|at| (bounds, at))
            });
            place_main(app, &window.as_ref().window(), at, attrs.maximized);
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
    let saved = attrs.bounds.and_then(|bounds| {
        land(label, bounds, monitors, Fallback::DefaultPlace).map(|at| (bounds, at))
    });
    let window = crate::commands::popout::build_runtime_window(app, label, saved.map(first_place))?;
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

// 팝아웃 빌더의 첫 자리 — 고른 모니터의 배율로 푼 논리 위치 · 논리 크기. 정확한 자리는 만든 뒤 [`place`] 가 놓는다
//   (`build_runtime_window` 의 `at` 인자).
fn first_place((bounds, at): (WindowBounds, Landing)) -> (LogicalPosition<f64>, LogicalSize<f64>) {
    (
        LogicalPosition::new(f64::from(at.x) / at.scale, f64::from(at.y) / at.scale),
        LogicalSize::new(bounds.w(), bounds.h()),
    )
}

/// 이미 있는 창에 저장된 보통 자리를 입힌다. 없거나 어느 모니터에도 안 걸치는 자리면 창은 지금 자리에 남는다.
fn place_saved(window: &Window, bounds: Option<WindowBounds>, monitors: &[MonitorArea]) {
    if let Some(bounds) = bounds {
        if let Some(at) = land(window.label(), bounds, monitors, Fallback::StayPut) {
            place(window, bounds, at);
        }
    }
}

/// 저장된 자리를 버리면 창이 어디로 가나 — [`land`] 의 로그 문구만 가른다.
#[derive(Debug, Clone, Copy)]
pub(super) enum Fallback {
    /// 새로 여는 팝아웃 — label 의 기본 자리로 연다.
    DefaultPlace,
    /// 이미 있는 창(main · 트리) — 지금 자리에 남는다.
    StayPut,
}

/// [`locate`] 에 로그 한 줄을 더한 것 — 어느 모니터에도 안 걸쳐 버린 자리를 info 로 남긴다.
pub(super) fn land(
    label: &str,
    bounds: WindowBounds,
    monitors: &[MonitorArea],
    fallback: Fallback,
) -> Option<Landing> {
    let at = locate(bounds, monitors);
    if at.is_none() {
        match fallback {
            Fallback::DefaultPlace => tracing::info!(
                module = "state",
                label,
                "저장된 창 자리가 어느 모니터에도 안 걸쳐 버렸다 — 기본 자리로 연다"
            ),
            Fallback::StayPut => tracing::info!(
                module = "state",
                label,
                "저장된 창 자리가 어느 모니터에도 안 걸쳐 버렸다 — 창은 지금 자리에 남는다"
            ),
        }
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

// 보통 자리를 입힌 뒤에 부른다 — tao 의 위치 세터는 최대화 표식을 내린다(F8). `false` = 못 입혔다.
fn maximize(window: &Window) -> bool {
    match window.maximize() {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(module = "state", label = window.label(), error = %e, "저장된 최대화를 못 입혔다");
            false
        }
    }
}

// main 에 보통 자리와 최대화를 입힌다 — 계약은 `RestoreWindows::place_main`(부팅 ⑧ 도 같은 길). 보임 여부를 못
//   읽으면 보이는 것으로 친다 — 보이는 창에 미루면 보이기 경로가 다시 불릴 때까지 최대화가 안 된다.
fn place_main(
    app: &AppHandle,
    window: &Window,
    at: Option<(WindowBounds, Landing)>,
    maximized: bool,
) {
    let place_at = || {
        if let Some((bounds, at)) = at {
            place(window, bounds, at);
        }
    };
    let deferred = app.try_state::<DeferredMaximize>();
    if !maximized {
        // 미룸을 먼저 거둔다 — 그래야 아래 자리 입히기가 낸 `Moved` · `Resized` 가 적힌다([`record`]).
        if deferred.is_some_and(|deferred| deferred.take(MAIN_WINDOW_LABEL)) {
            tracing::debug!(
                module = "state",
                "사본의 main 이 최대화가 아니라 미뤄 둔 최대화를 거둔다"
            );
        }
        place_at();
        // [미검증 — 숨은 main 이 최대화돼 있으면 tao 의 풀기(`SW_RESTORE`)가 창을 보였다 숨긴다(최대화와 같은 길 —
        //   [`DeferredMaximize`]). GUI 실측 몫]
        if matches!(window.is_maximized(), Ok(true)) {
            if let Err(e) = window.unmaximize() {
                tracing::warn!(module = "state", error = %e, "사본대로 main 의 최대화를 풀지 못했다");
            }
        }
        return;
    }
    let hidden = matches!(window.is_visible(), Ok(false));
    match deferred {
        Some(deferred) if hidden => {
            deferred.defer(MAIN_WINDOW_LABEL);
            place_at();
            // 보임을 읽은 뒤 트레이 「보이기」가 끼어들어 미룬 최대화를 이미 입혔으면, 그 뒤의 위치 세터가 최대화를
            //   내렸다(F8) — 보이게 된 main 에는 여기서 입히고 미룸을 거둔다. 그 보이기가 이 뒤에 오면 미룸이 남아
            //   그쪽이 입힌다. 못 입혔으면 미룸을 남긴다([`DeferredMaximize::apply`] 와 같은 까닭).
            if matches!(window.is_visible(), Ok(true)) && maximize(window) {
                deferred.take(MAIN_WINDOW_LABEL);
            }
        }
        Some(deferred) => {
            deferred.apply_now(MAIN_WINDOW_LABEL, place_at, || maximize(window));
        }
        None => {
            place_at();
            maximize(window);
        }
    }
}

/// 보이기 경로가 창을 보인 직후 부른다 — 그 창의 미룬 최대화가 있으면 입히고 지금 자리를 적는다
/// ([`DeferredMaximize`]). 미룬 동안 적지 않은 자리를 여기서 메운다. 못 입혔으면 미룸을 남기고 적지 않는다
/// ([`DeferredMaximize::apply`]).
pub fn apply_deferred_maximize(window: &Window) {
    if window
        .app_handle()
        .try_state::<DeferredMaximize>()
        .is_some_and(|deferred| deferred.apply(window.label(), || maximize(window)))
    {
        record(window);
    }
}

/// 창이 소멸했다(`WindowEvent::Destroyed`) — 그 창의 미룬 최대화를 거둔다. 팝아웃 label 은 재사용되지 않아 남겨도 다른
/// 창에 입혀지지는 않지만, 거두지 않으면 집합이 숨긴 채 만들었다 거둔 창마다 자란다.
pub fn forget_deferred_maximize(app: &AppHandle, label: &str) {
    if app
        .try_state::<DeferredMaximize>()
        .is_some_and(|deferred| deferred.take(label))
    {
        tracing::debug!(
            module = "state",
            label,
            "미룬 최대화를 입히기 전에 창이 소멸했다"
        );
    }
}

/// 런타임 복원 수락의 OS 창 포트 — Tauri 실물(TRD S21-storage §6-7 ② ④). 부팅 전용인 [`restore_windows`] ·
/// [`open_restored_popouts`] 를 쓰지 않는다 — 자리 판정([`locate`])과 입히기([`place`])만 같이 쓴다.
pub struct TauriRestoreWindows {
    app: AppHandle,
}

impl TauriRestoreWindows {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }

    fn window(&self, label: &str) -> Option<Window> {
        let window = self.app.get_webview_window(label);
        if window.is_none() {
            tracing::debug!(
                module = "state",
                label,
                "창이 없어 복원한 자리 · 보임을 입히지 않는다"
            );
        }
        window.map(|window| window.as_ref().window())
    }
}

impl RestoreWindows for TauriRestoreWindows {
    fn app_has_focus(&self) -> bool {
        foreground_is_ours(&self.app)
    }

    fn monitors(&self) -> Vec<MonitorArea> {
        monitors(&self.app)
    }

    fn open_hidden(
        &self,
        label: &str,
        at: Option<(WindowBounds, Landing)>,
        maximized: bool,
    ) -> Result<(), String> {
        let deferred = if maximized {
            let deferred = self.app.try_state::<DeferredMaximize>();
            match &deferred {
                Some(deferred) => deferred.defer(label),
                None => tracing::debug!(
                    module = "state",
                    label,
                    "미룬 최대화 표가 등록돼 있지 않아 그 창의 최대화를 입히지 않는다"
                ),
            }
            deferred
        } else {
            None
        };
        let window = match crate::commands::popout::build_hidden_runtime_window(
            &self.app,
            label,
            at.map(first_place),
        ) {
            Ok(window) => window.as_ref().window(),
            Err(e) => {
                if let Some(deferred) = deferred {
                    deferred.take(label);
                }
                return Err(e);
            }
        };
        if let Some((bounds, at)) = at {
            place(&window, bounds, at);
            // 만든 뒤 트레이 「보이기」가 끼어들어 미룬 최대화를 이미 입혔으면 방금의 위치 세터가 그 최대화를 내렸다(F8)
            //   — 보이게 된 창에는 여기서 다시 입힌다(`place_main` 과 같은 까닭). 그 보이기가 이 뒤에 오면 미룸이 남아
            //   그쪽이 입힌다.
            if maximized && matches!(window.is_visible(), Ok(true)) && maximize(&window) {
                if let Some(deferred) = &deferred {
                    deferred.take(label);
                }
            }
        }
        Ok(())
    }

    fn visibility(&self, label: &str) -> Option<bool> {
        let window = self.app.get_webview_window(label)?;
        Some(window.is_visible().unwrap_or_else(|e| {
            tracing::debug!(
                module = "state",
                label,
                error = %e,
                "창의 보임 여부를 못 읽어 보이는 것으로 친다"
            );
            true
        }))
    }

    fn place_main(&self, at: Option<(WindowBounds, Landing)>, maximized: bool) {
        if let Some(window) = self.window(MAIN_WINDOW_LABEL) {
            place_main(&self.app, &window, at, maximized);
        }
    }

    fn place(&self, label: &str, bounds: WindowBounds, at: Landing) {
        if let Some(window) = self.window(label) {
            place(&window, bounds, at);
        }
    }

    fn set_shown(&self, label: &str, shown: bool) {
        let Some(window) = self.window(label) else {
            return;
        };
        crate::tray::actions::with_usage_visibility(&self.app, |usage| {
            crate::tray::actions::for_each_ui_window([label], shown, usage, |label| {
                let result = if shown { window.show() } else { window.hide() };
                match result {
                    // 보인 창에만 — 숨은 창을 최대화하면 보였다 숨는다.
                    Ok(()) if shown => {
                        apply_deferred_maximize(&window);
                        true
                    }
                    Ok(()) => true,
                    Err(e) => {
                        tracing::warn!(module = "state", label, shown, error = %e, "복원한 창의 보임을 바꾸지 못했다");
                        false
                    }
                }
            });
        });
    }

    fn record_placement(&self, label: &str) {
        if let Some(window) = self.window(label) {
            record(&window);
        }
    }

    fn focus(&self, label: &str) {
        if let Some(window) = self.window(label) {
            if let Err(e) = window.set_focus() {
                tracing::debug!(module = "state", label, error = %e, "복원 뒤 창에 포커스를 주지 못했다");
            }
        }
    }

    fn destroy(&self, label: &str) -> Result<(), String> {
        crate::commands::popout::try_destroy_window(&self.app, label)
    }
}

// 우리 프로세스가 지금 전경 창을 쥐었나 — 그래야 tao 의 포커스 주기가 가짜 Alt 없이 선다(`force_window_active` 는
//   `SetForegroundWindow` 가 거절될 때만 Alt 를 쏜다 — tao 0.35.3).
// ★Windows 에서는 창의 포커스 게터(`is_focused`)를 쓰지 않는다★ — wry 0.55.1 이 창의 `WM_SETFOCUS` 마다 포커스를
//   웹뷰로 넘겨(`MoveFocus`) 창은 곧바로 `WM_KILLFOCUS` 를 받고, tao 0.35.3 의 그 값(`is_active && is_focused`)은
//   웹뷰가 키보드 포커스를 쥔 동안 `false` 다. tauri-runtime-wry 2.11.3 도 그래서 창의 포커스 사건을 tao 가 아니라
//   웹뷰의 GotFocus · LostFocus 에서 만든다. [미검증 — 소스 독해. GUI 실측 몫]
// 그 밖의 OS 는 그 게터를 그대로 쓴다. [미검증 — 그 갈래는 Windows 빌드 · CI 에서 컴파일되지 않는다]
fn foreground_is_ours(app: &AppHandle) -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::UI::WindowsAndMessaging::{
            GetForegroundWindow, GetWindowThreadProcessId,
        };
        let _ = app;
        let mut pid = 0u32;
        // SAFETY: 인자 없는 조회 하나와, 살아 있는 지역 변수에 PID 를 쓰는 조회 하나다. 전경 창이 없거나 그 사이
        //   사라졌으면 PID 가 0 으로 남는다.
        unsafe {
            let foreground = GetForegroundWindow();
            if !foreground.is_invalid() {
                GetWindowThreadProcessId(foreground, Some(&mut pid));
            }
        }
        pid != 0 && pid == std::process::id()
    }
    #[cfg(not(windows))]
    {
        app.webview_windows()
            .values()
            .any(|window| matches!(window.is_focused(), Ok(true)))
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
/// 잘못 적히지 않는다(`PopupCounter`). 최대화를 미뤄 둔 동안의 창도 적지 않는다([`DeferredMaximize`]).
/// 규칙(최소화 · 최대화)은 [`WindowAttrs`] 의 `observe`.
pub fn record(window: &Window) {
    let label = window.label();
    let app = window.app_handle();
    if app
        .try_state::<DeferredMaximize>()
        .is_some_and(|deferred| deferred.is_pending(label))
    {
        tracing::debug!(
            module = "state",
            label,
            "최대화를 보일 때로 미뤄 둔 동안이라 이번 자리는 적지 않는다"
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
        assert!(!deferred.is_pending(MAIN_WINDOW_LABEL));
        assert!(
            !deferred.take(MAIN_WINDOW_LABEL),
            "미룬 것이 없으면 입히지 않는다"
        );

        deferred.defer(MAIN_WINDOW_LABEL);
        assert!(
            deferred.is_pending(MAIN_WINDOW_LABEL),
            "서 있는 동안 main 의 자리를 적지 않는다"
        );
        assert!(deferred.take(MAIN_WINDOW_LABEL));
        assert!(!deferred.is_pending(MAIN_WINDOW_LABEL));
        assert!(
            !deferred.take(MAIN_WINDOW_LABEL),
            "다음 보이기는 다시 최대화하지 않는다"
        );
    }

    #[test]
    fn a_deferred_maximize_that_fails_to_apply_stays_pending_for_the_next_show() {
        let deferred = DeferredMaximize::default();
        assert!(
            !deferred.apply("slot-popup-5", || panic!("미룬 것이 없으면 입히지 않는다")),
            "미룬 것이 없다"
        );

        deferred.defer("slot-popup-5");
        assert!(
            !deferred.apply("slot-popup-5", || false),
            "못 입혔으면 자리를 적지 않는다"
        );
        assert!(
            deferred.is_pending("slot-popup-5"),
            "못 입힌 미룸은 남는다 — 그동안 보통 자리를 「최대화 아님」으로 적지 않는다"
        );

        assert!(
            deferred.apply("slot-popup-5", || true),
            "다음 보이기가 다시 입힌다"
        );
        assert!(!deferred.is_pending("slot-popup-5"));
        assert!(
            !deferred.apply("slot-popup-5", || panic!("한 번만 입힌다")),
            "입힌 뒤에는 다시 입히지 않는다"
        );
    }

    #[test]
    fn a_visible_window_keeps_its_deferral_until_the_maximize_lands() {
        let deferred = DeferredMaximize::default();
        assert!(!deferred.apply_now(
            MAIN_WINDOW_LABEL,
            || assert!(
                deferred.is_pending(MAIN_WINDOW_LABEL),
                "자리를 입히는 동안 main 의 자리를 적지 않는다"
            ),
            || false,
        ));
        assert!(
            deferred.is_pending(MAIN_WINDOW_LABEL),
            "못 입혔으면 남는다 — 복원 뒤의 자리 기록이 main 을 「최대화 아님」으로 적지 않는다"
        );

        assert!(
            deferred.apply(MAIN_WINDOW_LABEL, || true),
            "다음 보이기가 다시 입힌다"
        );
        assert!(!deferred.is_pending(MAIN_WINDOW_LABEL));

        assert!(deferred.apply_now(MAIN_WINDOW_LABEL, || {}, || true));
        assert!(
            !deferred.is_pending(MAIN_WINDOW_LABEL),
            "입혔으면 거둔다 — main 의 자리가 다시 적힌다"
        );
    }

    #[test]
    fn a_show_that_takes_the_deferral_mid_placement_does_not_stop_the_maximize() {
        let deferred = DeferredMaximize::default();
        let mut maximized = false;
        assert!(!deferred.apply_now(
            MAIN_WINDOW_LABEL,
            // 트레이 「보이기」가 끼어들어 미룬 최대화를 입히고 거뒀다 — 그 뒤의 위치 세터가 그 최대화를 내린다(F8).
            || assert!(deferred.apply(MAIN_WINDOW_LABEL, || true)),
            || {
                maximized = true;
                false
            },
        ));
        assert!(maximized, "거둬졌어도 입히기를 부른다");
        assert!(
            deferred.is_pending(MAIN_WINDOW_LABEL),
            "못 입혔으면 거둬졌던 미룸을 다시 세운다"
        );
    }

    #[test]
    fn deferred_maximizes_are_kept_per_window() {
        let deferred = DeferredMaximize::default();
        deferred.defer("slot-popup-3");
        deferred.defer("slot-popup-4");

        assert!(
            !deferred.is_pending(MAIN_WINDOW_LABEL),
            "다른 창은 그대로 적힌다"
        );
        assert!(deferred.take("slot-popup-3"), "보인 창 하나만 입힌다");
        assert!(!deferred.is_pending("slot-popup-3"));
        assert!(
            deferred.is_pending("slot-popup-4"),
            "아직 숨은 창의 미룸은 남는다"
        );
        assert!(
            deferred.take("slot-popup-4"),
            "창 소멸도 같은 거두기다 — 거둔 뒤 집합이 빈다"
        );
        assert!(deferred.labels().is_empty());
    }
}
