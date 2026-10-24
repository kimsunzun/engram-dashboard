//! 창 상태 서비스(TRD S26-window-chrome §5-2 · §5-4 · §5-5 · §6-2 · §6-3) — 최소화 · 최대화 · 원래 크기 · 앞으로 내기 ·
//! 닫기 합류 · 비었으면 닫기 · 창 목록의 OS 상태. 버스 동사와 invoke 껍데기가 같은 함수에 떨어진다(ADR-0081).
//!
//! - ★Tauri 0★ — OS 창에는 포트 둘로만 닿는다: [`WindowControl`](서비스가 부른다 · 실물은 동작 하나를 메인 스레드 일
//!   하나로 돈다)과 [`WindowOps`](그 일 안에서 쓰는 창 하나의 OS 조작). 판정은 순수 [`plan`] 이고 일의 몸통은
//!   [`run_action`] · [`show_one_no_steal`] 이라 창 없이 잰다.
//! - ★읽기 · 판정 · 입히기는 메인 스레드 일 하나다★(§5-2) — 첫 화면 보이기가 메인 스레드에서 미룬 최대화 표를 읽고 거둔다.
//!   다른 스레드에서 상태를 읽고 메인 스레드에서 미룸을 적으면 그 틈에 보이기가 지나가 미룸이 홀로 남고, 그동안 그 창의
//!   자리 기록(`state::placement::record`)이 멈춘다.
//! - ★레이아웃 락을 쥔 채 포트를 부르지 않는다★(`super::apply` 머리 「락 규율」) — 실물의 일이 같은 락을 다시 잡는다(재진입
//!   안 되는 뮤텍스). 서비스는 창 인자를 락 안에서 풀고 놓은 뒤 포트를 부른다. [`run_action`] 은 모델의 최대화 표식을 적을
//!   때만 락을 잠깐 잡고 쥔 채 [`WindowOps`] 를 부르지 않는다.
//! - ★첫 화면 대기 창(첫 화면 문에 올려진 창)을 드러내지 않는다★(§2-4 불변식 5) — 그 창을 보이는 것은 첫 화면 문과 사람의
//!   트레이 누름뿐이다.

use super::apply::{self, SubscriptionSync, WindowHost};
use super::manager::{WindowLabel, MAIN_WINDOW_LABEL};
use super::window_arg::{self, WindowArgError};
use super::LayoutState;

/// 창 하나에 거는 상태 동작 — 닫기는 [`close`] 다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowAction {
    Minimize,
    Maximize,
    Unmaximize,
    Raise,
}

/// 모든 창 보이기의 갈래(TRD §6-2) — 둘 다 예열 창을 건너뛴다.
///
/// - `TrayClick` = 사람의 손(트레이 아이콘 누름 · 단일 인스턴스의 두 번째 실행)뿐이다. 첫 화면 대기 창의 기다림도 거두고
///   보이며 창마다 포커스를 준다(main 이 마지막 — 누름이 전경 권한을 준다).
/// - `NoSteal` = 그 밖 전부(버스 `window.showAll` · 숨은 창의 `window.raise` · invoke `show_main_ui`). 첫 화면 대기 창을
///   건너뛰고 창마다 [`show_one_no_steal`] 이다 — 포커스는 우리 프로세스가 이미 전경을 쥐었을 때 main 에만 준다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activation {
    TrayClick,
    NoSteal,
}

/// 동작 하나의 답 — 버스 `window.maximize` · `window.unmaximize` 는 `deferred` 를, `window.raise` 는 나머지 셋을 싣는다.
///
/// - `deferred` = 보이지 않는 창이라 OS 창을 건드리지 않고 바랄 최대화 상태(최대화 · 원래 크기)를 적어 뒀다 — 그 창이 보일
///   때 입혀진다.
/// - `pending` = 첫 화면 대기 창이라 아무것도 하지 않았다(첫 화면 문이 곧 보인다).
/// - `shown_all` = 숨은 창이라 먼저 모든 창을 보였다.
/// - `focused` = 끝난 뒤 그 창이 OS 전경 창이다 — 사실 그대로다(남의 앱이 전경이면 맨 위로만 올리고 거짓 · AC9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Applied {
    pub deferred: bool,
    pub pending: bool,
    pub shown_all: bool,
    pub focused: bool,
}

/// 창 하나의 OS 상태 순간값. 모두 거짓(`Default`) = 그 창의 OS 창을 못 읽었다(`window.list` 행 — TRD §6-3).
///
/// - `maximized` = 최소화된 동안은 거짓이다 — tao 0.35.3 이 최소화의 `WM_SIZE` 에서 최대화 깃발을 내린다
///   (`platform_impl/windows/event_loop.rs`). 그래서 최소화된 창이 펴면 최대화로 돌아올지는 이 값으로 모른다.
/// - `focused` = 그 창이 OS 전경 창이다 — Windows 에서는 Tauri `is_focused` 가 아니다(웹뷰가 키보드 포커스를 쥔 동안 거짓
///   이다 — TRD §6-2 「앞 창 판정」).
/// - `awaiting_first_paint` = 첫 화면 문에 올려져 있다(`first_paint` — 그 문이 곧 보인다). 이 창은 숨은 채다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OsWindowState {
    pub visible: bool,
    pub minimized: bool,
    pub maximized: bool,
    pub focused: bool,
    pub awaiting_first_paint: bool,
}

/// 보이지 않는 창의 최소화를 거절하는 까닭 — [`Plan::Refuse`] 의 문구가 된다(버스 `CONFLICT`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Unseen {
    #[error("아직 첫 화면 전인 창 — 첫 화면을 그리면 곧 보인다")]
    AwaitingFirstPaint,
    #[error("숨은 창 — 먼저 window.raise 또는 window.showAll")]
    Hidden,
}

/// [`plan`] 의 판정 — [`run_action`] 이 그대로 입힌다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// 이미 그 상태다.
    Noop,
    Minimize,
    /// 보이는 창 — OS 로 최대화하고 적어 둔 바람을 거둔다. [`Self::Unmaximize`] 도 같다.
    Maximize,
    Unmaximize,
    /// 보이지 않는 창 — 바랄 최대화 상태(참 = 최대화)를 적고 모델 표식을 그 값으로 둔다. OS 창은 건드리지 않는다(그 창을
    /// 보이는 길이 입힌다).
    Defer(bool),
    /// 창이 이미 그 최대화 상태로 보이거나 보일 창이다 — 적어 둔 바람을 거두고 모델 표식을 그 값으로 둔다. 거두지 않으면
    /// 먼저 적은 반대 바람이 보일 때 입혀져 이 동작을 뒤집는다(보이는 창에도 입히기가 실패해 남은 바람이 있을 수 있다). OS
    /// 창은 건드리지 않는다.
    Settle(bool),
    /// 보이는 창을 앞으로(§6-2 ⓐ–ⓒ).
    Raise,
    /// 숨은 창 — 포커스를 빼앗지 않고 모든 창을 보인 뒤 그 창을 앞으로.
    ShowAllThenRaise,
    /// 첫 화면 대기 창 — 아무것도 하지 않는다.
    Pending,
    Refuse(Unseen),
}

/// 판정 표(TRD S26-window-chrome §5-2) — 창은 첫 화면 대기 → 보임 → 숨김 차례로 가른다. 표 밖에서 정한 것:
///
/// - 보이는 창이 이미 그 최대화 상태면 OS 창은 건드리지 않고 바람만 거둔다([`Plan::Settle`]). 보이는 최소화 창의 원래
///   크기는 무동작이다 — 최소화된 동안 `maximized` 가 거짓이다([`OsWindowState`] · 먼저 `window.raise`).
/// - 보이지 않는 창의 최대화 · 원래 크기는 거절하지 않는다 — OS 창을 건드리면 tao 가 창을 드러내므로([`WindowOps`]) 바랄
///   상태를 적고 보일 때 입힌다. 이미 그 상태로 보일 것이 분명하면(최소화 아님) 적어 둔 바람만 거둔다. 최소화된 창에는 늘
///   적는다 — 펴면 어느 크기로 돌아올지 모른다.
pub fn plan(action: WindowAction, s: OsWindowState) -> Plan {
    let unseen = if s.awaiting_first_paint {
        Some(Unseen::AwaitingFirstPaint)
    } else if s.visible {
        None
    } else {
        Some(Unseen::Hidden)
    };
    let shows_maximized = s.maximized && !s.minimized;
    let shows_normal = !s.maximized && !s.minimized;
    match (action, unseen) {
        (WindowAction::Minimize, None) if s.minimized => Plan::Noop,
        (WindowAction::Minimize, None) => Plan::Minimize,
        (WindowAction::Minimize, Some(why)) => Plan::Refuse(why),
        (WindowAction::Maximize, _) if shows_maximized => Plan::Settle(true),
        (WindowAction::Maximize, None) => Plan::Maximize,
        (WindowAction::Maximize, Some(_)) => Plan::Defer(true),
        (WindowAction::Unmaximize, None) if shows_maximized => Plan::Unmaximize,
        (WindowAction::Unmaximize, None) if shows_normal => Plan::Settle(false),
        (WindowAction::Unmaximize, None) => Plan::Noop,
        (WindowAction::Unmaximize, Some(_)) if shows_normal => Plan::Settle(false),
        (WindowAction::Unmaximize, Some(_)) => Plan::Defer(false),
        (WindowAction::Raise, None) => Plan::Raise,
        (WindowAction::Raise, Some(Unseen::AwaitingFirstPaint)) => Plan::Pending,
        (WindowAction::Raise, Some(Unseen::Hidden)) => Plan::ShowAllThenRaise,
    }
}

/// 메인 스레드 일 하나 안에서 쓰는 창 하나의 OS 조작 — [`run_action`] · [`show_one_no_steal`] 이 부른다. 실물(Tauri 어댑터)은
/// Tauri 창 · platform `window` · 첫 화면 문(`first_paint`) · 미룬 최대화 표(`state::placement::DeferredMaximize` — label
/// 마다 바랄 최대화 상태 하나)를 부른다(TRD §6-2).
///
/// - 모든 메서드는 그 일 안에서(메인 스레드), 레이아웃 락을 쥐지 않은 채 불린다. 실물은 그 락을 잠깐 잡아도 된다(쥔 채 OS 를
///   부르지 않는 한 — 자리 기록이 그렇다).
/// - 그 label 의 OS 창이 없으면 `Result` 메서드는 `Err`, 나머지는 무동작이다.
pub trait WindowOps {
    /// 지금 상태 — `None` = 그 label 의 OS 창이 없다. 칸의 뜻은 [`OsWindowState`].
    fn read(&self, label: &str) -> Option<OsWindowState>;
    /// 보이는 창에만 불린다(숨은 창의 최소화 · 최대화 · 풀기는 tao 가 창을 드러낸다).
    fn minimize(&self, label: &str) -> Result<(), String>;
    /// 보이는 창에만 불린다.
    fn maximize(&self, label: &str) -> Result<(), String>;
    /// 보이는 창에만 불린다.
    fn unmaximize(&self, label: &str) -> Result<(), String>;
    /// 바랄 최대화 상태를 적는다(먼저 적은 것을 덮는다) — 그 창을 보이는 길이 [`Self::apply_deferred`] 로 입힌다. 모델의
    /// 최대화 표식은 [`run_action`] 이 적는다 — 실물은 모델을 건드리지 않는다.
    fn defer_maximized(&self, label: &str, maximized: bool);
    /// 적어 둔 바람을 거둔다(없으면 무동작). 모델 표식은 [`Self::defer_maximized`] 와 같다.
    fn drop_deferred(&self, label: &str);
    /// ⓐ 활성화 없이 z 만 맨 위 — 숨은 창은 숨은 채다(platform `place_window(.., ZOrder::Top, show: false, activate: false)`).
    fn raise_without_activation(&self, label: &str);
    /// ⓑ Tauri `show` — ★날 Win32 로 보이지 않는다★: tao 의 보임 깃발이 거짓으로 남으면 다음 숨기기가 아무것도 하지
    /// 않는다. 첫 화면 대기 창에는 불리지 않는다.
    fn show(&self, label: &str) -> Result<(), String>;
    /// ⓑ Tauri `unminimize`.
    fn unminimize(&self, label: &str) -> Result<(), String>;
    /// ⓑ 보이기 · 펴기 뒤에 불린다 — 바람이 적혀 있으면 입히고(참 = 최대화 · 거짓 = 원래 크기) 거둔다. 못 입히면 남기고,
    /// 없으면 무동작이다. 실물의 다른 보이기 길(첫 화면 보이기 · 트레이 보이기)도 보이기 · 펴기 뒤에 같은 일을 해야 한다 —
    /// 펴기 전에 입히면 최소화된 창의 거짓 바람을 잃는다([`OsWindowState`] 의 `maximized` · `bring_up` 머리).
    fn apply_deferred(&self, label: &str);
    /// ⓒ 우리 프로세스가 지금 전경을 쥐었나 — `state::placement::foreground_is_ours`.
    fn foreground_is_ours(&self) -> bool;
    /// ⓒ Tauri `set_focus` — [`Self::foreground_is_ours`] 가 참일 때만 불린다(남의 앱이 전경이면 tao 가 가짜 Alt 로 빼앗는다).
    fn set_focus(&self, label: &str);
    /// 포커스를 빼앗지 않고 모든 창을 보인다 — [`Activation::NoSteal`] 의 몸통(`tray::actions` 의 창 차례 · 예열 창 건너뛰기
    /// 그대로, 창마다 [`show_one_no_steal`] · main 만 `focus`). 숨은 창의 앞으로 내기가 같은 일 안에서 부른다.
    fn show_all_no_steal(&self);
}

/// 창 상태 포트 — 서비스가 부른다. 실물(`commands::window_ctl`)은 `apply` · `show_all` · `hide_all` 을 각각 메인 스레드 일
/// 하나(`run_on_main_thread`)로 돌고 그 답을 기다린다(TRD §5-2 · §6-2). 버스는 이 포트를 부르는 동사를 블로킹 풀에서 돈다
/// (`super::commands` 의 `offloaded_handler`).
///
/// - ★레이아웃 락을 쥔 채 부르지 않는다★ — 그 일이 같은 락을 다시 잡는다(재진입 안 되는 뮤텍스 — 메인 스레드에서 부르면
///   같은 스레드가 두 번 잠그고, 다른 스레드면 일이 락을 기다리는 동안 부른 쪽이 일을 기다린다).
/// - 메인 스레드에서 불러도 교착은 아니다 — tauri-runtime-wry 2.11.3 `send_user_message` 가 메인 스레드에서 넘긴 일을 그
///   자리에서 돈다. 그래서 실물은 답을 버퍼 있는 채널(`mpsc::channel()` · `sync_channel(1)`)로 받는다 — 용량 0 채널이면 그
///   자리에서 도는 일의 보내기가 아직 오지 않은 받기를 영영 기다린다.
/// - 실물은 `recv_timeout` 으로 기다린다 — 메인 스레드가 멈추면 부른 스레드를 붙들지 않고 시한 뒤 `apply` = `Err` ·
///   `show_all` = 거짓 · `hide_all` = 무동작으로 끝난다.
pub trait WindowControl: Send + Sync {
    /// [`run_action`] 을 메인 스레드 일 하나로 돈다. `Err` = 그 문구(버스 `CONFLICT`).
    fn apply(&self, label: &str, action: WindowAction) -> Result<Applied, String>;
    /// 모든 창을 보인다 — 실물 = `tray::actions` 의 보이기(`how` 갈래) 하나. 답 = 그 뒤 main 이 OS 전경 창이다.
    fn show_all(&self, how: Activation) -> bool;
    /// 모든 창을 숨긴다 — 실물 = `tray::actions::hide_main_ui`(ADR-0229 — main 의 닫기 · 트레이 숨기기와 한 경로).
    fn hide_all(&self);
    /// `window.list` 몫 순간값 — `None` = 그 label 의 OS 창이 없다.
    fn state(&self, label: &str) -> Option<OsWindowState>;
}

/// 창 `label` 에 `action` 을 건다 — 읽기 → [`plan`] → 입히기. [`WindowControl::apply`] 의 실물이 메인 스레드 일 하나 안에서
/// 부르는 몸통이다(TRD §5-2).
///
/// - `Err` = OS 창이 없다 · 보이지 않는 창의 최소화라 거절했다([`Unseen`] 의 문구) · OS 호출이 실패했다 · 모델에 그 창이
///   없거나 락이 오염돼 최대화 표식을 못 적었다(그때 미룬 최대화 표는 그대로다).
/// - 바람 적기 · 거두기는 모델 표식을 먼저 적고 표를 바꾼다. 그 쓰기에 `layout` 락을 잠깐 잡으므로 ★부르는 쪽은 그 락을
///   쥐지 않는다★.
pub fn run_action<O: WindowOps + ?Sized>(
    ops: &O,
    layout: &LayoutState,
    label: &str,
    action: WindowAction,
) -> Result<Applied, String> {
    let state = ops.read(label).ok_or_else(|| no_os_window(label))?;
    match plan(action, state) {
        Plan::Noop => Ok(Applied::default()),
        Plan::Minimize => ops.minimize(label).map(|()| Applied::default()),
        Plan::Maximize => {
            ops.maximize(label)?;
            ops.drop_deferred(label);
            Ok(Applied::default())
        }
        Plan::Unmaximize => {
            ops.unmaximize(label)?;
            ops.drop_deferred(label);
            Ok(Applied::default())
        }
        Plan::Defer(maximized) => {
            set_model_maximized(layout, label, maximized)?;
            ops.defer_maximized(label, maximized);
            Ok(Applied {
                deferred: true,
                ..Applied::default()
            })
        }
        Plan::Settle(maximized) => {
            set_model_maximized(layout, label, maximized)?;
            ops.drop_deferred(label);
            Ok(Applied::default())
        }
        Plan::Raise => {
            bring_up(ops, label, state, true)?;
            Ok(Applied {
                focused: is_foreground(ops, label),
                ..Applied::default()
            })
        }
        Plan::ShowAllThenRaise => {
            ops.show_all_no_steal();
            let state = ops.read(label).ok_or_else(|| no_os_window(label))?;
            // 모든 창을 보이는 사이 첫 화면 문에 올려졌다 — 그 문이 보인다(불변식 5).
            if state.awaiting_first_paint {
                return Ok(Applied {
                    shown_all: true,
                    pending: true,
                    ..Applied::default()
                });
            }
            bring_up(ops, label, state, true)?;
            Ok(Applied {
                shown_all: true,
                focused: is_foreground(ops, label),
                ..Applied::default()
            })
        }
        Plan::Pending => Ok(Applied {
            pending: true,
            ..Applied::default()
        }),
        Plan::Refuse(why) => Err(why.to_string()),
    }
}

// ADR-0299
/// 포커스를 빼앗지 않는 보이기의 창 하나 몫(TRD §6-2 ⓐ–ⓒ) — [`Activation::NoSteal`] 의 모든 창 보이기가 창마다 부른다.
/// 첫 화면 대기 창 · OS 창이 없는 창은 건드리지 않고 `false`. 보인 뒤 적어 둔 바람을 입힌다([`WindowOps::apply_deferred`]).
/// `focus` = ⓒ 를 청한다(우리 프로세스가 지금 전경을 쥐었을 때만 포커스를 준다). 보이기 · 펴기가 실패하면 로그를 남기고
/// `false`.
pub fn show_one_no_steal<O: WindowOps + ?Sized>(ops: &O, label: &str, focus: bool) -> bool {
    let Some(state) = ops.read(label) else {
        return false;
    };
    if state.awaiting_first_paint {
        return false;
    }
    match bring_up(ops, label, state, focus) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(label, error = %e, "포커스를 빼앗지 않는 보이기 — 창을 보이지 못했다");
            false
        }
    }
}

// ADR-0299 · TRD §6-2 ⓐ → ⓑ → ⓒ. ⓑ 는 펴기 뒤에 바람을 입힌다 — 입힌 뒤 펴면(`SW_RESTORE`) 최소화 전 크기로 돌아가 바람이
//   덮인다. 첫 화면 대기 창은 부르는 쪽이 이미 걸렀어도 여기서 한 번 더 건드리지 않는다(불변식 5).
fn bring_up<O: WindowOps + ?Sized>(
    ops: &O,
    label: &str,
    state: OsWindowState,
    focus: bool,
) -> Result<(), String> {
    if state.awaiting_first_paint {
        return Ok(());
    }
    ops.raise_without_activation(label);
    if !state.visible {
        ops.show(label)?;
    }
    if state.minimized {
        ops.unminimize(label)?;
    }
    ops.apply_deferred(label);
    if focus && ops.foreground_is_ours() {
        ops.set_focus(label);
    }
    Ok(())
}

fn is_foreground<O: WindowOps + ?Sized>(ops: &O, label: &str) -> bool {
    ops.read(label).is_some_and(|state| state.focused)
}

fn no_os_window(label: &str) -> String {
    format!("window 없음: {label} — OS 창이 열려 있지 않다")
}

fn set_model_maximized(layout: &LayoutState, label: &str, maximized: bool) -> Result<(), String> {
    let mut mgr = layout.0.lock().map_err(|e| e.to_string())?;
    mgr.set_window_maximized(label, maximized)
        .map_err(|e| e.to_string())
}

// ── 서비스 ───────────────────────────────────────────────────────────────────

/// 창 인자 · 동작이 못 선 까닭 — 버스가 코드로 옮긴다(`Arg` = [`WindowArgError`] 의 코드 · `NotApplied` = `CONFLICT`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WindowCtlError {
    #[error(transparent)]
    Arg(#[from] WindowArgError),
    #[error("{0}")]
    NotApplied(String),
}

/// 창 인자 `window` 를 풀어(TRD §4-5) 그 창에 `action` 을 건다. 인자는 레이아웃 락 안에서 풀고 포트는 놓은 뒤 부른다 — 그
/// 사이 창이 닫혀 OS 창이 없으면 포트가 거절한다.
pub fn act(
    state: &LayoutState,
    control: &dyn WindowControl,
    window: &str,
    action: WindowAction,
) -> Result<Applied, WindowCtlError> {
    let label = resolve(state, window)?;
    control
        .apply(&label, action)
        .map_err(WindowCtlError::NotApplied)
}

/// `window.close` 의 결말.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseOutcome {
    /// 팝아웃을 닫았다.
    Closed,
    /// main 이라 닫지 않고 모든 창을 숨겼다.
    HiddenAll,
    /// 비었으면 닫기 — 그 창에 탭이 있거나 이미 닫혀 그대로 뒀다(오류가 아니다).
    Kept,
}

/// 창을 닫는다(TRD §5-4) — 차례: ① 숫자만인 인자는 `NumberGiven`(`if_empty` 여도) ② `if_empty` 면 [`close_if_empty`](인자 =
/// 영속 id · 해석기를 거치지 않는다 — 그 창은 대개 이미 모델에 없다) ③ 인자를 풀어 main 이면 모든 창 숨기기, 팝아웃이면
/// `apply::close_window`.
pub fn close(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    host: &dyn WindowHost,
    control: &dyn WindowControl,
    window: &str,
    if_empty: bool,
) -> Result<CloseOutcome, WindowCtlError> {
    window_arg::reject_number(window)?;
    if if_empty {
        return close_if_empty(state, host, window).map_err(WindowCtlError::NotApplied);
    }
    let label = resolve(state, window)?;
    if label == MAIN_WINDOW_LABEL {
        // ADR-0229
        control.hide_all();
        return Ok(CloseOutcome::HiddenAll);
    }
    apply::close_window(state, subs, host, &label).map_err(WindowCtlError::NotApplied)?;
    Ok(CloseOutcome::Closed)
}

/// 0탭 자가닫힘(TRD §5-5) — 영속 id `window_id` 의 창이 모델에 있으면(그 창엔 탭이 1 개 이상이다) `Kept`, 모델에 없고 닫히는
/// 창이면 그 OS 창이 열려 있을 때 닫고 `Closed`, 둘 다 아니면 `Kept`. 모델 판정이 한 락 안이라 그 사이 탭이 들어오는 틈이
/// 없다(들어오면 첫 갈래다). 영속 id 로만 고르므로 다른 창에 닿지 않는다. `Err` = 락 오염.
pub fn close_if_empty(
    state: &LayoutState,
    host: &dyn WindowHost,
    window_id: &str,
) -> Result<CloseOutcome, String> {
    let window_id = window_id.trim();
    let closing = {
        let mgr = state.0.lock().map_err(|e| e.to_string())?;
        if window_arg::resolve(&mgr, window_id).is_ok() {
            return Ok(CloseOutcome::Kept);
        }
        mgr.closing_label_for(window_id)
    };
    match closing {
        Some(label) if host.is_open(&label) => {
            host.close(&label);
            Ok(CloseOutcome::Closed)
        }
        _ => Ok(CloseOutcome::Kept),
    }
}

/// `window.list` 의 행 하나(TRD §6-3) — `window` = label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedWindow {
    pub window_id: String,
    pub window: WindowLabel,
    pub number: u32,
    pub main: bool,
    pub os: OsWindowState,
}

/// 모델 안 창 전부 — 번호 순. 모델 행은 락 안에서 뜨고 OS 상태는 락 밖에서 읽어 짝짓는다(순간값). OS 창이 없는 행은 상태가
/// 모두 거짓이다. 닫히는 창은 모델에 없어 싣지 않는다. `Err` = 락 오염.
pub fn list(state: &LayoutState, control: &dyn WindowControl) -> Result<Vec<ListedWindow>, String> {
    let mut rows: Vec<(WindowLabel, String, u32)> = {
        let mgr = state.0.lock().map_err(|e| e.to_string())?;
        mgr.windows
            .iter()
            .map(|(label, w)| (label.clone(), w.window_id.clone(), w.number))
            .collect()
    };
    rows.sort_by_key(|(_, _, number)| *number);
    Ok(rows
        .into_iter()
        .map(|(label, window_id, number)| ListedWindow {
            os: control.state(&label).unwrap_or_default(),
            main: label == MAIN_WINDOW_LABEL,
            window_id,
            window: label,
            number,
        })
        .collect())
}

pub(crate) fn resolve(state: &LayoutState, window: &str) -> Result<WindowLabel, WindowCtlError> {
    let mgr = state
        .0
        .lock()
        .map_err(|e| WindowCtlError::NotApplied(e.to_string()))?;
    Ok(window_arg::resolve(&mgr, window)?)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::{BTreeMap, BTreeSet, VecDeque};
    use std::sync::{Mutex, TryLockError};

    use super::*;
    use crate::layout::apply::{MoveFailed, SpareClaim, WindowPlace};
    use crate::layout::manager::ViewManager;

    const POP: &str = "slot-popup-0a1b2c3d-1";
    const POP2: &str = "slot-popup-0a1b2c3d-2";

    fn os(visible: bool, minimized: bool, maximized: bool, awaiting: bool) -> OsWindowState {
        OsWindowState {
            visible,
            minimized,
            maximized,
            focused: false,
            awaiting_first_paint: awaiting,
        }
    }

    // ── plan(§5-2 표) ──

    #[test]
    fn plan_follows_the_table_for_every_kind_of_window_and_action() {
        use WindowAction::{Maximize, Minimize, Raise, Unmaximize};
        let visible = |min, max| os(true, min, max, false);
        let hidden = |min, max| os(false, min, max, false);
        let awaiting = |min, max| os(false, min, max, true);
        let awaiting_refused = Plan::Refuse(Unseen::AwaitingFirstPaint);
        let hidden_refused = Plan::Refuse(Unseen::Hidden);
        // (동작, 상태(최소화, 최대화), 판정) — 총 27 행. 최소화된 창은 최대화를 거짓으로 읽는다(`OsWindowState`) — 둘 다
        // 참인 상태는 싣지 않는다.
        let rows = [
            // 보임
            (Minimize, visible(false, false), Plan::Minimize),
            (Minimize, visible(false, true), Plan::Minimize),
            (Minimize, visible(true, false), Plan::Noop),
            (Maximize, visible(false, false), Plan::Maximize),
            (Maximize, visible(false, true), Plan::Settle(true)),
            (Maximize, visible(true, false), Plan::Maximize),
            (Unmaximize, visible(false, false), Plan::Settle(false)),
            (Unmaximize, visible(false, true), Plan::Unmaximize),
            (Unmaximize, visible(true, false), Plan::Noop),
            (Raise, visible(false, false), Plan::Raise),
            (Raise, visible(true, false), Plan::Raise),
            // 첫 화면 대기
            (Minimize, awaiting(false, false), awaiting_refused),
            (Maximize, awaiting(false, false), Plan::Defer(true)),
            (Maximize, awaiting(false, true), Plan::Settle(true)),
            (Unmaximize, awaiting(false, false), Plan::Settle(false)),
            (Unmaximize, awaiting(false, true), Plan::Defer(false)),
            (Raise, awaiting(false, false), Plan::Pending),
            // 숨김(모두 숨기기 · --hidden)
            (Minimize, hidden(false, false), hidden_refused),
            (Minimize, hidden(true, false), hidden_refused),
            (Maximize, hidden(false, false), Plan::Defer(true)),
            (Maximize, hidden(true, false), Plan::Defer(true)),
            (Maximize, hidden(false, true), Plan::Settle(true)),
            (Unmaximize, hidden(false, false), Plan::Settle(false)),
            (Unmaximize, hidden(true, false), Plan::Defer(false)),
            (Unmaximize, hidden(false, true), Plan::Defer(false)),
            (Raise, hidden(false, false), Plan::ShowAllThenRaise),
            (Raise, hidden(true, false), Plan::ShowAllThenRaise),
        ];
        for (action, state, want) in rows {
            assert_eq!(plan(action, state), want, "{action:?} {state:?}");
            let focused = OsWindowState {
                focused: true,
                ..state
            };
            assert_eq!(plan(action, focused), want, "앞 창이어도 같다 — {action:?}");
        }
    }

    // 첫 화면 문에 올려진 창은 보인다는 깃발이 서 있어도 대기 창이다 — 그 창을 보이는 것은 그 문이다.
    #[test]
    fn plan_treats_a_held_window_as_awaiting_even_if_it_reads_visible() {
        let held = os(true, false, false, true);
        assert_eq!(
            plan(WindowAction::Minimize, held),
            Plan::Refuse(Unseen::AwaitingFirstPaint)
        );
        assert_eq!(plan(WindowAction::Raise, held), Plan::Pending);
    }

    // ── 메인 스레드 줄 흉내(§5-2 「시험」) ──

    #[derive(Debug, Clone, Copy, Default)]
    struct FakeWindow {
        visible: bool,
        minimized: bool,
        /// 최대화로 놓였다 — 최소화된 동안도 남아 펴면 최대화로 돌아온다. 읽기는 tao 처럼 최소화된 동안 거짓을 낸다.
        maximized: bool,
        held: bool,
    }

    /// 창 · 미룬 최대화 표 · 전경을 쥔 OS 대역. 숨은 창에 OS 최소화 · 최대화 · 풀기가 오거나 대기 창을 보이면 터진다 — 실물이면
    /// 창이 드러나는 자리다.
    #[derive(Default)]
    struct FakeOps {
        windows: RefCell<BTreeMap<String, FakeWindow>>,
        deferred: RefCell<BTreeMap<String, bool>>,
        /// OS 전경 창 — `None` = 남의 앱.
        foreground: RefCell<Option<String>>,
        /// 모든 창 보이기가 처음에 이 창을 첫 화면 문에 다시 올린다.
        hold_on_show_all: RefCell<Option<String>>,
        /// 바람 입히기가 실패한다 — 바람은 남는다.
        fail_apply: RefCell<bool>,
        calls: RefCell<Vec<String>>,
    }

    impl FakeOps {
        fn with(windows: &[(&str, FakeWindow)]) -> Self {
            let ops = FakeOps::default();
            for (label, window) in windows {
                ops.windows.borrow_mut().insert(label.to_string(), *window);
            }
            ops
        }

        fn window(&self, label: &str) -> FakeWindow {
            self.windows.borrow()[label]
        }

        fn note(&self, call: &str, label: &str) {
            self.calls.borrow_mut().push(format!("{call}:{label}"));
        }

        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }

        fn deferred(&self) -> Vec<(String, bool)> {
            self.deferred
                .borrow()
                .iter()
                .map(|(label, maximized)| (label.clone(), *maximized))
                .collect()
        }

        fn change(&self, label: &str, call: &str, edit: impl FnOnce(&mut FakeWindow)) {
            self.note(call, label);
            let mut windows = self.windows.borrow_mut();
            let window = windows.get_mut(label).expect("OS 창");
            assert!(
                window.visible,
                "숨은 창 {label} 에 OS {call} — 창이 드러난다"
            );
            edit(window);
        }

        // 첫 화면 보이기(`state::placement::show_placed`) — 대기를 풀고 보인 뒤 적어 둔 바람을 입히고 거둔다.
        fn first_paint_show(&self, label: &str) {
            self.note("first_paint", label);
            let mut windows = self.windows.borrow_mut();
            let window = windows.get_mut(label).expect("OS 창");
            if !window.held {
                return;
            }
            window.held = false;
            window.visible = true;
            if let Some(maximized) = self.deferred.borrow_mut().remove(label) {
                window.maximized = maximized;
            }
        }
    }

    impl WindowOps for FakeOps {
        fn read(&self, label: &str) -> Option<OsWindowState> {
            let window = *self.windows.borrow().get(label)?;
            Some(OsWindowState {
                visible: window.visible,
                minimized: window.minimized,
                maximized: window.maximized && !window.minimized,
                focused: self.foreground.borrow().as_deref() == Some(label),
                awaiting_first_paint: window.held,
            })
        }

        fn minimize(&self, label: &str) -> Result<(), String> {
            self.change(label, "minimize", |w| w.minimized = true);
            Ok(())
        }

        fn maximize(&self, label: &str) -> Result<(), String> {
            self.change(label, "maximize", |w| {
                w.maximized = true;
                w.minimized = false;
            });
            Ok(())
        }

        fn unmaximize(&self, label: &str) -> Result<(), String> {
            self.change(label, "unmaximize", |w| {
                w.maximized = false;
                w.minimized = false;
            });
            Ok(())
        }

        fn defer_maximized(&self, label: &str, maximized: bool) {
            self.note("defer", label);
            self.deferred
                .borrow_mut()
                .insert(label.to_string(), maximized);
        }

        fn drop_deferred(&self, label: &str) {
            self.note("drop_deferred", label);
            self.deferred.borrow_mut().remove(label);
        }

        fn raise_without_activation(&self, label: &str) {
            self.note("top", label);
        }

        fn show(&self, label: &str) -> Result<(), String> {
            self.note("show", label);
            let mut windows = self.windows.borrow_mut();
            let window = windows.get_mut(label).expect("OS 창");
            assert!(!window.held, "첫 화면 대기 창 {label} 을 드러냈다");
            window.visible = true;
            Ok(())
        }

        fn unminimize(&self, label: &str) -> Result<(), String> {
            self.note("unminimize", label);
            self.windows
                .borrow_mut()
                .get_mut(label)
                .expect("OS 창")
                .minimized = false;
            Ok(())
        }

        fn apply_deferred(&self, label: &str) {
            if *self.fail_apply.borrow() {
                self.note("apply_deferred_failed", label);
                return;
            }
            let Some(maximized) = self.deferred.borrow_mut().remove(label) else {
                return;
            };
            self.change(label, "apply_deferred", |w| w.maximized = maximized);
        }

        fn foreground_is_ours(&self) -> bool {
            self.foreground.borrow().is_some()
        }

        fn set_focus(&self, label: &str) {
            self.note("focus", label);
            *self.foreground.borrow_mut() = Some(label.to_string());
        }

        // 트레이 차례 그대로 — 팝아웃 먼저, main 이 마지막이고 main 만 포커스를 청한다.
        fn show_all_no_steal(&self) {
            self.note("show_all", "*");
            if let Some(label) = self.hold_on_show_all.borrow_mut().take() {
                let mut windows = self.windows.borrow_mut();
                let window = windows.get_mut(&label).expect("OS 창");
                window.held = true;
                window.visible = false;
            }
            let mut labels: Vec<String> = self.windows.borrow().keys().cloned().collect();
            labels.sort_by_key(|label| label == MAIN_WINDOW_LABEL);
            for label in labels {
                show_one_no_steal(self, &label, label == MAIN_WINDOW_LABEL);
            }
        }
    }

    /// 메인 스레드 한 줄 — 넣은 차례대로 하나씩 돈다.
    enum Job {
        FirstPaintShow(&'static str),
        Act(&'static str, WindowAction),
        ShowAllNoSteal,
    }

    #[derive(Default)]
    struct MainThread(VecDeque<Job>);

    impl MainThread {
        fn push(&mut self, job: Job) -> &mut Self {
            self.0.push_back(job);
            self
        }

        /// 줄을 비울 때까지 돈다 — 답 = `Act` 일의 답(넣은 차례).
        fn run(&mut self, ops: &FakeOps, layout: &LayoutState) -> Vec<Result<Applied, String>> {
            let mut answers = Vec::new();
            while let Some(job) = self.0.pop_front() {
                match job {
                    Job::FirstPaintShow(label) => ops.first_paint_show(label),
                    Job::Act(label, action) => answers.push(run_action(ops, layout, label, action)),
                    Job::ShowAllNoSteal => ops.show_all_no_steal(),
                }
            }
            answers
        }
    }

    fn layout_with_popouts(labels: &[&str]) -> LayoutState {
        let state = LayoutState::new();
        {
            let mut mgr = state.0.lock().unwrap();
            for label in labels {
                mgr.create_window(label).unwrap();
            }
        }
        state
    }

    fn model_maximized(state: &LayoutState, label: &str) -> bool {
        state
            .0
            .lock()
            .unwrap()
            .window_attrs(label)
            .unwrap()
            .maximized
    }

    const HELD: FakeWindow = FakeWindow {
        visible: false,
        minimized: false,
        maximized: false,
        held: true,
    };
    const HIDDEN: FakeWindow = FakeWindow {
        visible: false,
        minimized: false,
        maximized: false,
        held: false,
    };
    const SHOWN: FakeWindow = FakeWindow {
        visible: true,
        minimized: false,
        maximized: false,
        held: false,
    };

    // (a) 첫 화면 보이기 먼저 — 최대화는 보인 창으로 판정돼 그 자리에서 입혀지고 미룸 표는 비어 있다.
    #[test]
    fn a_maximize_queued_after_the_first_paint_show_applies_at_once() {
        let ops = FakeOps::with(&[(MAIN_WINDOW_LABEL, SHOWN), (POP, HELD)]);
        let layout = layout_with_popouts(&[POP]);
        let answers = MainThread::default()
            .push(Job::FirstPaintShow(POP))
            .push(Job::Act(POP, WindowAction::Maximize))
            .run(&ops, &layout);
        assert_eq!(answers, [Ok(Applied::default())]);
        assert!(ops.window(POP).maximized);
        assert!(
            ops.deferred().is_empty(),
            "미룸이 남으면 자리 기록이 멈춘다"
        );
    }

    // (b) 최대화 먼저 — 바람에 적히고(모델 표식도) 뒤의 첫 화면 보이기가 입히고 거둔다.
    #[test]
    fn a_maximize_queued_before_the_first_paint_show_is_deferred_and_the_show_applies_it() {
        let ops = FakeOps::with(&[(MAIN_WINDOW_LABEL, SHOWN), (POP, HELD)]);
        let layout = layout_with_popouts(&[POP]);
        let mut main_thread = MainThread::default();
        let answers = main_thread
            .push(Job::Act(POP, WindowAction::Maximize))
            .run(&ops, &layout);
        assert_eq!(
            answers,
            [Ok(Applied {
                deferred: true,
                ..Applied::default()
            })]
        );
        assert_eq!(ops.deferred(), [(POP.to_string(), true)]);
        assert!(model_maximized(&layout, POP));
        assert!(!ops.window(POP).visible, "대기 창은 숨은 채다");

        main_thread
            .push(Job::FirstPaintShow(POP))
            .run(&ops, &layout);
        assert!(ops.window(POP).visible && ops.window(POP).maximized);
        assert!(ops.deferred().is_empty());
    }

    // 최대화로 지어진(바람이 선) 대기 창 — 원래 크기가 먼저면 바람만 거둬 보통 크기로 보이고, 보이기가 먼저면 OS 로 푼다.
    // 두 차례 모두 OS 창은 이미 그 크기라 미루지 않는다.
    #[test]
    fn unmaximize_drops_the_wish_before_the_show_and_unmaximizes_after_it() {
        for show_first in [false, true] {
            let ops = FakeOps::with(&[(MAIN_WINDOW_LABEL, SHOWN), (POP, HELD)]);
            ops.deferred.borrow_mut().insert(POP.to_string(), true);
            let layout = layout_with_popouts(&[POP]);
            layout
                .0
                .lock()
                .unwrap()
                .set_window_maximized(POP, true)
                .unwrap();
            let mut main_thread = MainThread::default();
            if show_first {
                main_thread.push(Job::FirstPaintShow(POP));
            }
            main_thread.push(Job::Act(POP, WindowAction::Unmaximize));
            if !show_first {
                main_thread.push(Job::FirstPaintShow(POP));
            }
            let answers = main_thread.run(&ops, &layout);
            assert_eq!(answers, [Ok(Applied::default())], "show_first={show_first}");
            assert!(ops.window(POP).visible, "show_first={show_first}");
            assert!(!ops.window(POP).maximized, "show_first={show_first}");
            assert!(ops.deferred().is_empty(), "show_first={show_first}");
            if !show_first {
                assert!(!model_maximized(&layout, POP), "모델 표식도 내린다");
            }
        }
    }

    // 숨은 창 — 최소화는 거절, 최대화는 미룸, 앞으로 내기는 포커스를 빼앗지 않는 모든 창 보이기를 먼저 하고 그 보이기가
    // 미룬 최대화를 입힌다. 전경은 우리 창(POP2)이 쥐었다 — 그래서 포커스를 청한다.
    #[test]
    fn a_hidden_window_refuses_minimize_defers_maximize_and_raise_shows_all_first() {
        let ops = FakeOps::with(&[(MAIN_WINDOW_LABEL, HIDDEN), (POP, HIDDEN), (POP2, SHOWN)]);
        *ops.foreground.borrow_mut() = Some(POP2.to_string());
        let layout = layout_with_popouts(&[POP, POP2]);
        let answers = MainThread::default()
            .push(Job::Act(POP, WindowAction::Minimize))
            .push(Job::Act(POP, WindowAction::Maximize))
            .run(&ops, &layout);
        assert_eq!(answers[0], Err(Unseen::Hidden.to_string()));
        assert_eq!(
            answers[1],
            Ok(Applied {
                deferred: true,
                ..Applied::default()
            })
        );
        assert_eq!(ops.deferred(), [(POP.to_string(), true)]);
        ops.calls.borrow_mut().clear();

        let answers = MainThread::default()
            .push(Job::Act(POP, WindowAction::Raise))
            .run(&ops, &layout);
        assert_eq!(
            answers,
            [Ok(Applied {
                shown_all: true,
                focused: true,
                ..Applied::default()
            })]
        );
        assert!(ops.window(MAIN_WINDOW_LABEL).visible);
        assert!(ops.window(POP).visible && ops.window(POP).maximized);
        assert!(ops.deferred().is_empty());
        assert_eq!(
            ops.calls(),
            [
                "show_all:*".to_string(),
                format!("top:{POP}"),
                format!("show:{POP}"),
                format!("apply_deferred:{POP}"),
                format!("top:{POP2}"),
                "top:main".to_string(),
                "show:main".to_string(),
                "focus:main".to_string(),
                format!("top:{POP}"),
                format!("focus:{POP}"),
            ],
            "모든 창 보이기는 main 에만 포커스를 청하고, 그 뒤 그 창을 맨 위로 올려 포커스를 청한다"
        );
    }

    // 최대화한 뒤 최소화하고 숨긴 창 — 최대화를 거짓으로 읽어도 원래 크기는 미뤄 적고, 보이기가 펴기(최대화로 돌아온다) 뒤에
    // 그 바람을 입혀 보통 크기로 끝난다.
    #[test]
    fn unmaximize_of_a_hidden_maximized_then_minimized_window_is_deferred_and_the_show_unmaximizes_it(
    ) {
        let parked = FakeWindow {
            minimized: true,
            maximized: true,
            ..HIDDEN
        };
        let ops = FakeOps::with(&[(MAIN_WINDOW_LABEL, HIDDEN), (POP, parked)]);
        let layout = layout_with_popouts(&[POP]);
        layout
            .0
            .lock()
            .unwrap()
            .set_window_maximized(POP, true)
            .unwrap();
        let mut main_thread = MainThread::default();
        let answers = main_thread
            .push(Job::Act(POP, WindowAction::Unmaximize))
            .run(&ops, &layout);
        assert_eq!(
            answers,
            [Ok(Applied {
                deferred: true,
                ..Applied::default()
            })]
        );
        assert_eq!(ops.deferred(), [(POP.to_string(), false)]);
        assert!(!model_maximized(&layout, POP));

        main_thread.push(Job::ShowAllNoSteal).run(&ops, &layout);
        let shown = ops.window(POP);
        assert!(
            shown.visible && !shown.minimized && !shown.maximized,
            "{shown:?}"
        );
        assert!(ops.deferred().is_empty());
    }

    // 보이지 않는 창에 반대 바람이 먼저 적혀 있으면, OS 창이 이미 그 크기인 뒤 동작이 그 바람을 거둔다 — 남기면 보일 때 뒤
    // 동작이 뒤집힌다.
    #[test]
    fn a_later_action_on_a_hidden_window_overrides_an_earlier_wish() {
        // (숨은 창의 OS 최대화, 앞 동작, 뒤 동작) — 끝에 보인 창의 최대화 = 뒤 동작
        for (os_maximized, first, last) in [
            (false, WindowAction::Maximize, WindowAction::Unmaximize),
            (true, WindowAction::Unmaximize, WindowAction::Maximize),
        ] {
            let hidden = FakeWindow {
                maximized: os_maximized,
                ..HIDDEN
            };
            let ops = FakeOps::with(&[(MAIN_WINDOW_LABEL, HIDDEN), (POP, hidden)]);
            let layout = layout_with_popouts(&[POP]);
            let want = last == WindowAction::Maximize;
            let answers = MainThread::default()
                .push(Job::Act(POP, first))
                .push(Job::Act(POP, last))
                .push(Job::ShowAllNoSteal)
                .run(&ops, &layout);
            assert_eq!(
                answers,
                [
                    Ok(Applied {
                        deferred: true,
                        ..Applied::default()
                    }),
                    Ok(Applied::default())
                ],
                "{last:?}"
            );
            assert!(ops.deferred().is_empty(), "{last:?}");
            assert_eq!(model_maximized(&layout, POP), want, "{last:?}");
            assert!(ops.window(POP).visible, "{last:?}");
            assert_eq!(ops.window(POP).maximized, want, "{last:?}");
        }
    }

    // 보이는 창에 남은 바람(입히기가 실패해 남았다)은 이미 그 크기인 동작도 거둔다 — 남기면 다음 숨기기 · 보이기가 입혀
    // 그 동작을 뒤집는다.
    #[test]
    fn a_no_op_action_on_a_visible_window_drops_a_stale_wish_and_writes_the_model() {
        let ops = FakeOps::with(&[(MAIN_WINDOW_LABEL, SHOWN), (POP, SHOWN)]);
        ops.deferred.borrow_mut().insert(POP.to_string(), true);
        let layout = layout_with_popouts(&[POP]);
        layout
            .0
            .lock()
            .unwrap()
            .set_window_maximized(POP, true)
            .unwrap();
        let answers = MainThread::default()
            .push(Job::Act(POP, WindowAction::Unmaximize))
            .run(&ops, &layout);
        assert_eq!(answers, [Ok(Applied::default())]);
        assert!(ops.deferred().is_empty());
        assert!(!model_maximized(&layout, POP));
        assert!(!ops.window(POP).maximized, "OS 창은 그대로다");
    }

    // 숨은 최대화 창의 원래 크기 바람이 보이기에서 입혀지지 못하고 남은 뒤, 보이는 창의 최대화(이미 최대화 — 무동작)가 그
    // 바람을 거둔다. 그 뒤 숨기고 다시 보여도 원래 크기로 뒤집히지 않는다.
    #[test]
    fn a_wish_left_by_a_failed_apply_is_dropped_by_a_later_visible_action() {
        let hidden_maximized = FakeWindow {
            maximized: true,
            ..HIDDEN
        };
        let ops = FakeOps::with(&[(MAIN_WINDOW_LABEL, HIDDEN), (POP, hidden_maximized)]);
        let layout = layout_with_popouts(&[POP]);
        layout
            .0
            .lock()
            .unwrap()
            .set_window_maximized(POP, true)
            .unwrap();
        let mut main_thread = MainThread::default();
        main_thread
            .push(Job::Act(POP, WindowAction::Unmaximize))
            .run(&ops, &layout);
        assert_eq!(ops.deferred(), [(POP.to_string(), false)]);

        *ops.fail_apply.borrow_mut() = true;
        main_thread.push(Job::ShowAllNoSteal).run(&ops, &layout);
        assert!(ops.window(POP).visible && ops.window(POP).maximized);
        assert_eq!(
            ops.deferred(),
            [(POP.to_string(), false)],
            "못 입힌 바람은 남는다"
        );
        *ops.fail_apply.borrow_mut() = false;

        let answers = main_thread
            .push(Job::Act(POP, WindowAction::Maximize))
            .run(&ops, &layout);
        assert_eq!(answers, [Ok(Applied::default())]);
        assert!(ops.deferred().is_empty());
        assert!(model_maximized(&layout, POP));

        for window in ops.windows.borrow_mut().values_mut() {
            window.visible = false;
        }
        main_thread.push(Job::ShowAllNoSteal).run(&ops, &layout);
        assert!(ops.window(POP).visible && ops.window(POP).maximized);
        assert!(model_maximized(&layout, POP));
    }

    // 모든 창을 보이는 사이 그 창이 첫 화면 문에 올려지면 드러내지 않는다 — 그 문이 보인다.
    #[test]
    fn raise_of_a_hidden_window_held_again_while_showing_all_is_pending() {
        let ops = FakeOps::with(&[(MAIN_WINDOW_LABEL, HIDDEN), (POP, HIDDEN)]);
        *ops.hold_on_show_all.borrow_mut() = Some(POP.to_string());
        let layout = layout_with_popouts(&[POP]);
        let answers = MainThread::default()
            .push(Job::Act(POP, WindowAction::Raise))
            .run(&ops, &layout);
        assert_eq!(
            answers,
            [Ok(Applied {
                shown_all: true,
                pending: true,
                ..Applied::default()
            })]
        );
        let held = ops.window(POP);
        assert!(held.held && !held.visible, "대기 창은 손대지 않는다");
        assert!(
            !ops.calls().iter().any(|call| call.ends_with(POP)),
            "{:?}",
            ops.calls()
        );
        assert!(ops.window(MAIN_WINDOW_LABEL).visible);
    }

    // 숨은 창의 앞으로 내기도 남의 앱이 전경이면 보이기만 하고 포커스를 청하지 않는다 — 답은 사실 그대로 `focused: false`(AC9).
    #[test]
    fn raise_of_a_hidden_window_does_not_take_the_foreground_from_another_app() {
        let ops = FakeOps::with(&[(MAIN_WINDOW_LABEL, HIDDEN), (POP, HIDDEN)]);
        let layout = layout_with_popouts(&[POP]);
        let answers = MainThread::default()
            .push(Job::Act(POP, WindowAction::Raise))
            .run(&ops, &layout);
        assert_eq!(
            answers,
            [Ok(Applied {
                shown_all: true,
                ..Applied::default()
            })]
        );
        assert!(ops.window(POP).visible && ops.window(MAIN_WINDOW_LABEL).visible);
        assert!(
            !ops.calls().iter().any(|call| call.starts_with("focus:")),
            "{:?}",
            ops.calls()
        );
    }

    // LLM 의 모든 창 보이기는 첫 화면 대기 창을 건너뛴다 — 그 창은 곧 첫 화면 문이 보인다.
    #[test]
    fn no_steal_show_all_skips_a_window_waiting_for_its_first_paint() {
        let ops = FakeOps::with(&[(MAIN_WINDOW_LABEL, HIDDEN), (POP, HELD), (POP2, HIDDEN)]);
        let layout = layout_with_popouts(&[POP, POP2]);
        MainThread::default()
            .push(Job::ShowAllNoSteal)
            .run(&ops, &layout);
        assert!(ops.window(MAIN_WINDOW_LABEL).visible);
        assert!(ops.window(POP2).visible);
        let held = ops.window(POP);
        assert!(held.held && !held.visible, "대기 창은 손대지 않는다");
        assert!(
            !ops.calls().iter().any(|call| call.ends_with(POP)),
            "{:?}",
            ops.calls()
        );
        assert!(
            !ops.calls().iter().any(|call| call.starts_with("focus:")),
            "남의 앱이 전경이면 포커스를 주지 않는다"
        );

        MainThread::default()
            .push(Job::FirstPaintShow(POP))
            .run(&ops, &layout);
        assert!(ops.window(POP).visible);
    }

    // 남의 앱이 전경이면 맨 위로만 올리고 포커스를 청하지 않는다 — 답은 사실 그대로 `focused: false`(AC9).
    #[test]
    fn raise_does_not_take_the_foreground_from_another_app() {
        let minimized = FakeWindow {
            minimized: true,
            ..SHOWN
        };
        let ops = FakeOps::with(&[(MAIN_WINDOW_LABEL, SHOWN), (POP, minimized)]);
        let layout = layout_with_popouts(&[POP]);
        let answers = MainThread::default()
            .push(Job::Act(POP, WindowAction::Raise))
            .run(&ops, &layout);
        assert_eq!(answers, [Ok(Applied::default())]);
        assert_eq!(
            ops.calls(),
            [format!("top:{POP}"), format!("unminimize:{POP}")]
        );
        assert!(!ops.window(POP).minimized);
    }

    #[test]
    fn raise_of_a_window_waiting_for_its_first_paint_is_pending_and_touches_nothing() {
        let ops = FakeOps::with(&[(MAIN_WINDOW_LABEL, SHOWN), (POP, HELD)]);
        *ops.foreground.borrow_mut() = Some(MAIN_WINDOW_LABEL.to_string());
        let layout = layout_with_popouts(&[POP]);
        let answers = MainThread::default()
            .push(Job::Act(POP, WindowAction::Raise))
            .run(&ops, &layout);
        assert_eq!(
            answers,
            [Ok(Applied {
                pending: true,
                ..Applied::default()
            })]
        );
        assert!(ops.calls().is_empty(), "{:?}", ops.calls());
    }

    #[test]
    fn an_action_on_a_label_without_an_os_window_fails() {
        let ops = FakeOps::with(&[(MAIN_WINDOW_LABEL, SHOWN)]);
        let layout = layout_with_popouts(&[POP]);
        let err = run_action(&ops, &layout, POP, WindowAction::Maximize).unwrap_err();
        assert!(err.contains(POP), "{err}");
    }

    // 모델 표식을 못 적으면 바람을 적지도 거두지도 않는다 — 표와 모델이 갈리지 않는다.
    #[test]
    fn a_wish_for_a_window_missing_from_the_model_is_refused_without_touching_the_table() {
        let ops = FakeOps::with(&[(MAIN_WINDOW_LABEL, SHOWN), (POP, HIDDEN)]);
        let layout = LayoutState::new();
        assert!(run_action(&ops, &layout, POP, WindowAction::Maximize).is_err());
        assert!(ops.deferred().is_empty());
        ops.deferred.borrow_mut().insert(POP.to_string(), true);
        assert!(
            run_action(&ops, &layout, POP, WindowAction::Unmaximize).is_err(),
            "이미 보통 크기 — 바람만 거둘 자리"
        );
        assert_eq!(ops.deferred(), [(POP.to_string(), true)]);
    }

    // ── 서비스(창 인자 · 락 밖 포트) ──

    /// 기록형 창 상태 포트 — 부를 때마다 레이아웃 락 밖인지 본다.
    struct Control {
        layout: LayoutState,
        calls: Mutex<Vec<String>>,
        states: Mutex<BTreeMap<String, OsWindowState>>,
    }

    impl Control {
        fn new(layout: &LayoutState) -> Self {
            Control {
                layout: layout.clone(),
                calls: Mutex::new(Vec::new()),
                states: Mutex::new(BTreeMap::new()),
            }
        }

        fn outside(&self, call: String) {
            assert!(
                !matches!(self.layout.0.try_lock(), Err(TryLockError::WouldBlock)),
                "{call} 는 레이아웃 락 밖에서 불려야 한다"
            );
            self.calls.lock().unwrap().push(call);
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl WindowControl for Control {
        fn apply(&self, label: &str, action: WindowAction) -> Result<Applied, String> {
            self.outside(format!("apply:{label}:{action:?}"));
            Ok(Applied::default())
        }

        fn show_all(&self, how: Activation) -> bool {
            self.outside(format!("show_all:{how:?}"));
            false
        }

        fn hide_all(&self) {
            self.outside("hide_all".to_string());
        }

        fn state(&self, label: &str) -> Option<OsWindowState> {
            self.outside(format!("state:{label}"));
            self.states.lock().unwrap().get(label).copied()
        }
    }

    /// 창 포트 대역 — `opened` 에 든 창만 열려 있다(main 은 늘 열려 있다).
    struct Host {
        layout: LayoutState,
        opened: Mutex<BTreeSet<String>>,
        closed: Mutex<Vec<String>>,
    }

    impl Host {
        fn new(layout: &LayoutState) -> Self {
            Host {
                layout: layout.clone(),
                opened: Mutex::new(BTreeSet::new()),
                closed: Mutex::new(Vec::new()),
            }
        }

        fn open_os(&self, label: &str) {
            self.opened.lock().unwrap().insert(label.to_string());
        }

        fn closed(&self) -> Vec<String> {
            self.closed.lock().unwrap().clone()
        }
    }

    impl WindowHost for Host {
        fn open(&self, _label: &str) -> Result<(), String> {
            unreachable!("이 시험은 창을 열지 않는다")
        }

        fn open_at(&self, _label: &str, _at: &WindowPlace, _activate: bool) -> Result<(), String> {
            unreachable!("이 시험은 창을 열지 않는다")
        }

        fn close(&self, label: &str) {
            assert!(
                !matches!(self.layout.0.try_lock(), Err(TryLockError::WouldBlock)),
                "WindowHost::close 는 레이아웃 락 밖에서 불려야 한다"
            );
            self.opened.lock().unwrap().remove(label);
            self.closed.lock().unwrap().push(label.to_string());
        }

        fn is_open(&self, label: &str) -> bool {
            label == MAIN_WINDOW_LABEL || self.opened.lock().unwrap().contains(label)
        }

        fn move_window(&self, _label: &str, _at: &WindowPlace) -> Result<(), MoveFailed> {
            unreachable!("이 시험은 창을 옮기지 않는다")
        }

        fn claim_spare(&self) -> Option<SpareClaim> {
            None
        }

        fn committed(&self, _claim: &SpareClaim) {}

        fn release_spare(&self, _claim: &SpareClaim) {}
    }

    struct Subs;

    impl SubscriptionSync for Subs {
        fn resync(&self, _mgr: &ViewManager) {}
    }

    fn window_id(layout: &LayoutState, label: &str) -> String {
        layout.0.lock().unwrap().windows[label].window_id.clone()
    }

    #[test]
    fn act_resolves_the_window_argument_and_calls_the_port_outside_the_lock() {
        let layout = layout_with_popouts(&[POP]);
        let control = Control::new(&layout);
        let id = window_id(&layout, POP);
        act(&layout, &control, &id, WindowAction::Raise).unwrap();
        act(&layout, &control, MAIN_WINDOW_LABEL, WindowAction::Minimize).unwrap();
        assert_eq!(
            control.calls(),
            [
                format!("apply:{POP}:Raise"),
                "apply:main:Minimize".to_string()
            ]
        );

        assert_eq!(
            act(&layout, &control, "2", WindowAction::Maximize),
            Err(WindowCtlError::Arg(WindowArgError::NumberGiven))
        );
        assert_eq!(
            act(&layout, &control, "slot-popup-9", WindowAction::Maximize),
            Err(WindowCtlError::Arg(WindowArgError::NotFound(
                "slot-popup-9".to_string()
            )))
        );
        assert_eq!(
            control.calls().len(),
            2,
            "거절된 인자는 포트까지 가지 않는다"
        );
    }

    // ── 닫기 합류(§5-4) · 비었으면 닫기(§5-5) ──

    #[test]
    fn closing_main_hides_every_window_and_closing_a_popout_closes_it() {
        let layout = layout_with_popouts(&[POP]);
        let (control, host) = (Control::new(&layout), Host::new(&layout));
        host.open_os(POP);
        assert_eq!(
            close(&layout, &Subs, &host, &control, MAIN_WINDOW_LABEL, false),
            Ok(CloseOutcome::HiddenAll)
        );
        assert_eq!(control.calls(), ["hide_all"]);
        assert!(host.closed().is_empty(), "main 은 닫지 않는다");

        let id = window_id(&layout, POP);
        assert_eq!(
            close(&layout, &Subs, &host, &control, &id, false),
            Ok(CloseOutcome::Closed)
        );
        assert_eq!(host.closed(), [POP]);
        assert!(!layout.0.lock().unwrap().windows.contains_key(POP));
        layout.0.lock().unwrap().assert_number_invariants();
    }

    #[test]
    fn a_window_number_is_refused_before_if_empty_is_looked_at() {
        let layout = layout_with_popouts(&[POP]);
        let (control, host) = (Control::new(&layout), Host::new(&layout));
        for if_empty in [false, true] {
            assert_eq!(
                close(&layout, &Subs, &host, &control, " 2 ", if_empty),
                Err(WindowCtlError::Arg(WindowArgError::NumberGiven)),
                "if_empty={if_empty}"
            );
        }
        assert!(control.calls().is_empty() && host.closed().is_empty());
    }

    // 「모델에서만 빠지고 OS 창은 남은 창」 — 탭이 0 이 된 팝아웃의 상태다. 그 창만 영속 id 로 닫고, 모델에 있는 창 · 다른 id ·
    // OS 창이 이미 없는 창은 그대로 둔다(AC24 · AC32).
    #[test]
    fn if_empty_closes_only_the_closing_window_whose_os_window_is_still_open() {
        let layout = layout_with_popouts(&[POP, POP2]);
        let (control, host) = (Control::new(&layout), Host::new(&layout));
        host.open_os(POP);
        host.open_os(POP2);
        let (gone, kept) = (window_id(&layout, POP), window_id(&layout, POP2));
        let close_if_empty_of = |id: &str| close(&layout, &Subs, &host, &control, id, true);

        assert_eq!(
            close_if_empty_of(&kept),
            Ok(CloseOutcome::Kept),
            "모델에 있다"
        );
        assert_eq!(close_if_empty_of(MAIN_WINDOW_LABEL), Ok(CloseOutcome::Kept));
        layout.0.lock().unwrap().close_window(POP).unwrap();
        assert_eq!(
            close_if_empty_of(&uuid::Uuid::new_v4().to_string()),
            Ok(CloseOutcome::Kept),
            "다른 id"
        );
        assert!(host.closed().is_empty());

        assert_eq!(
            close_if_empty_of(&gone.to_uppercase()),
            Ok(CloseOutcome::Closed),
            "UUID 값이 같은 철자"
        );
        assert_eq!(host.closed(), [POP]);
        assert_eq!(
            close_if_empty_of(&gone),
            Ok(CloseOutcome::Kept),
            "OS 창이 이미 없다"
        );
        assert_eq!(host.closed(), [POP]);
        assert!(control.calls().is_empty(), "창 상태 포트를 부르지 않는다");
    }

    // ── 창 목록(§6-3) ──

    #[test]
    fn list_orders_rows_by_number_skips_closing_windows_and_reads_os_state_outside_the_lock() {
        let layout = layout_with_popouts(&[POP, POP2]);
        let control = Control::new(&layout);
        layout.0.lock().unwrap().close_window(POP).unwrap();
        let shown = OsWindowState {
            visible: true,
            maximized: true,
            focused: true,
            ..OsWindowState::default()
        };
        control
            .states
            .lock()
            .unwrap()
            .insert(POP2.to_string(), shown);

        let rows = list(&layout, &control).unwrap();
        let summary: Vec<(&str, u32, bool, OsWindowState)> = rows
            .iter()
            .map(|row| (row.window.as_str(), row.number, row.main, row.os))
            .collect();
        assert_eq!(
            summary,
            [
                (MAIN_WINDOW_LABEL, 1, true, OsWindowState::default()),
                (POP2, 3, false, shown),
            ],
            "OS 창을 못 읽은 행은 모두 거짓 · 닫히는 창(2)은 없다"
        );
        assert_eq!(rows[0].window_id, "main");
        assert_eq!(rows[1].window_id, window_id(&layout, POP2));
    }
}
