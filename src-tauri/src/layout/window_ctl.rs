//! Window state service (TRD S26-window-chrome §5-2 · §5-4 · §5-5 · §6-2 · §6-3) — minimize · maximize · restore size · bring to front ·
//! close join · close if empty · OS state of the window list. The bus verbs and the invoke shell land on the same function (ADR-0081).
//!
//! - ★Zero Tauri★ — it reaches the OS window only through two ports: [`WindowControl`] (the service calls it · the real one runs one action as one main-thread
//!   job) and [`WindowOps`] (OS operations on one window, used inside that job). The decision is the pure [`plan`] and the body of the job is
//!   [`run_action`] · [`show_one_no_steal`], so it is measured without a window.
//! - ★Reading · deciding · applying is one main-thread job★ (§5-2) — the first-paint show reads and withdraws the maximize table deferred on the main thread.
//!   If another thread reads the state and the main thread writes the deferral, the show can slip through in between, leaving the deferral alone, and meanwhile that window's
//!   placement recording (`state::placement::record`) stops.
//! - ★Do not call a port while holding the layout lock★ (`super::apply` header 「락 규율」) — the real job takes the same lock again (a non-reentrant
//!   mutex). The service resolves the window argument inside the lock, releases it, and then calls the port. [`run_action`] takes the lock briefly only to write the model's maximize marker
//!   and does not call [`WindowOps`] while holding it.
//! - ★Do not reveal a window waiting for first paint (a window placed on the first-paint gate)★ (§2-4 invariant 5) — what shows that window is only the first-paint gate and a person's
//!   tray click.

use super::apply::{self, SubscriptionSync, WindowHost};
use super::manager::{WindowLabel, MAIN_WINDOW_LABEL};
use super::window_arg::{self, WindowArgError};
use super::LayoutState;

/// State action applied to one window — closing is [`close`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowAction {
    Minimize,
    Maximize,
    Unmaximize,
    Raise,
}

/// Branches of show-all-windows (TRD §6-2) — both skip the prewarmed window.
///
/// - `TrayClick` = only a person's hand (tray icon click · the second launch of the single instance). It also withdraws the wait of a window waiting for first paint,
///   shows it and gives focus per window (main last — the click grants foreground permission).
/// - `NoSteal` = everything else (bus `window.showAll` · `window.raise` of a hidden window · invoke `show_main_ui`). It skips windows waiting for first paint
///   and runs [`show_one_no_steal`] per window — focus goes only to main, and only when our process already holds the foreground.
// ADR-0302 — window state command
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activation {
    TrayClick,
    NoSteal,
}

/// The answer of one action — bus `window.maximize` · `window.unmaximize` carry `deferred`, and `window.raise` carries the other three.
///
/// - `deferred` = the window is not visible, so without touching the OS window the desired maximize state (maximize · restore size) was written down — it is applied
///   when that window is shown.
/// - `pending` = it is a window waiting for first paint, so nothing was done (the first-paint gate will show it soon).
/// - `shown_all` = it is a hidden window, so all windows were shown first.
/// - `focused` = after finishing, that window is the OS foreground window — as is, in fact (if another app is in the foreground it is only raised to the top, and this is false · AC9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Applied {
    pub deferred: bool,
    pub pending: bool,
    pub shown_all: bool,
    pub focused: bool,
}

/// Snapshot of the OS state of one window. All false (`Default`) = the OS window could not be read (`window.list` row — TRD §6-3).
///
/// - `maximized` = false while minimized — tao 0.35.3 clears the maximize flag at the minimize `WM_SIZE`
///   (`platform_impl/windows/event_loop.rs`). So whether a minimized window comes back maximized when restored cannot be known from this value.
/// - `focused` = that window is the OS foreground window — on Windows it is not Tauri `is_focused` (it is false while the webview holds keyboard focus
///   — TRD §6-2 「앞 창 판정」).
/// - `awaiting_first_paint` = it is placed on the first-paint gate (`first_paint` — that gate will show it soon). This window stays hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OsWindowState {
    pub visible: bool,
    pub minimized: bool,
    pub maximized: bool,
    pub focused: bool,
    pub awaiting_first_paint: bool,
}

/// Why minimizing a non-visible window is rejected — becomes the message of [`Plan::Refuse`] (bus `CONFLICT`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Unseen {
    #[error("아직 첫 화면 전인 창 — 첫 화면을 그리면 곧 보인다")]
    AwaitingFirstPaint,
    #[error("숨은 창 — 먼저 window.raise 또는 window.showAll")]
    Hidden,
}

/// Decision of [`plan`] — [`run_action`] applies it as is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// It is already in that state.
    Noop,
    Minimize,
    /// A visible window — maximize via the OS and withdraw the desire written down. [`Self::Unmaximize`] is the same.
    Maximize,
    Unmaximize,
    /// A non-visible window — write down the desired maximize state (true = maximized) and set the model marker to that value. The OS window is not touched (the path
    /// that shows that window applies it).
    Defer(bool),
    /// The window is, or will be, shown in that maximize state already — withdraw the desire written down and set the model marker to that value. If not withdrawn,
    /// the opposite desire written earlier is applied when it is shown and reverses this action (a visible window can also have a leftover desire because applying failed). The OS
    /// window is not touched.
    Settle(bool),
    /// Bring a visible window to the front (§6-2 ⓐ–ⓒ).
    Raise,
    /// A hidden window — show all windows without stealing focus, then bring that window to the front.
    ShowAllThenRaise,
    /// A window waiting for first paint — do nothing.
    Pending,
    Refuse(Unseen),
}

/// Decision table (TRD S26-window-chrome §5-2) — windows are told apart in the order waiting for first paint → visible → hidden. What is decided outside the table:
///
/// - If a visible window is already in that maximize state, the OS window is not touched and only the desire is withdrawn ([`Plan::Settle`]). Restoring size of a visible minimized
///   window is a no-op — `maximized` is false while minimized ([`OsWindowState`] · `window.raise` first).
/// - Maximizing · restoring size of a non-visible window is not rejected — touching the OS window makes tao reveal the window ([`WindowOps`]), so the desired
///   state is written down and applied when shown. If it is clear it will be shown in that state already (not minimized), only the desire written down is withdrawn. For a minimized window it is always
///   written down — we do not know which size it will return to when restored.
// ADR-0302 — window state command
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

/// OS operations on one window used inside one main-thread job — [`run_action`] · [`show_one_no_steal`] call them. The real one (the Tauri adapter) calls
/// the Tauri window · platform `window` · the first-paint gate (`first_paint`) · the deferred maximize table (`state::placement::DeferredMaximize` — one desired
/// maximize state per label) (TRD §6-2).
///
/// - All methods are called inside that job (main thread), without holding the layout lock. The real one may take that lock briefly (as long as it does not
///   call the OS while holding it — the placement recording is like that).
/// - If there is no OS window for that label, `Result` methods give `Err` and the rest do nothing.
pub trait WindowOps {
    /// Current state — `None` = no OS window for that label. For the meaning of the fields see [`OsWindowState`].
    fn read(&self, label: &str) -> Option<OsWindowState>;
    /// Called only for visible windows (minimize · maximize · restore of a hidden window makes tao reveal the window).
    fn minimize(&self, label: &str) -> Result<(), String>;
    /// Called only for visible windows.
    fn maximize(&self, label: &str) -> Result<(), String>;
    /// Called only for visible windows.
    fn unmaximize(&self, label: &str) -> Result<(), String>;
    /// Writes down the desired maximize state (overwriting what was written earlier) — the path that shows that window applies it with [`Self::apply_deferred`]. The model's
    /// maximize marker is written by [`run_action`] — the real one does not touch the model.
    fn defer_maximized(&self, label: &str, maximized: bool);
    /// Withdraws the desire written down (no-op if none). The model marker is the same as [`Self::defer_maximized`].
    fn drop_deferred(&self, label: &str);
    /// ⓐ Only z to the top without activation — a hidden window stays hidden (platform `place_window(.., ZOrder::Top, show: false, activate: false)`).
    fn raise_without_activation(&self, label: &str);
    /// ⓑ Tauri `show` — ★do not show it with raw Win32★: if tao's visible flag stays false, the next hide does
    /// nothing. Not called for a window waiting for first paint.
    fn show(&self, label: &str) -> Result<(), String>;
    /// ⓑ Tauri `unminimize`.
    fn unminimize(&self, label: &str) -> Result<(), String>;
    /// ⓑ Called after showing · restoring — if a desire is written down, apply it (true = maximize · false = restore size) and withdraw it. If it cannot be applied, leave it, and
    /// if there is none, do nothing. The real one's other show paths (first-paint show · tray show) must also do the same after showing · restoring —
    /// applying before restoring loses the false desire of a minimized window (`maximized` of [`OsWindowState`] · `bring_up` header).
    fn apply_deferred(&self, label: &str);
    /// ⓒ Does our process hold the foreground now — `state::placement::foreground_is_ours`.
    fn foreground_is_ours(&self) -> bool;
    /// ⓒ Tauri `set_focus` — called only when [`Self::foreground_is_ours`] is true (if another app is in the foreground, tao takes it with a fake Alt).
    fn set_focus(&self, label: &str);
    /// Shows all windows without stealing focus — the body of [`Activation::NoSteal`] (the window order of `tray::actions` · skipping the prewarmed window as is,
    /// [`show_one_no_steal`] per window · only main gets `focus`). Bringing a hidden window to the front calls it inside the same job.
    fn show_all_no_steal(&self);
}

/// Window state port — the service calls it. The real one (`commands::window_ctl`) runs `apply` · `show_all` · `hide_all` each as one main-thread job
/// (`run_on_main_thread`) and waits for its answer (TRD §5-2 · §6-2). The bus runs the verbs that call this port in the blocking pool
/// (`offloaded_handler` of `super::commands`).
///
/// - ★Do not call it while holding the layout lock★ — that job takes the same lock again (a non-reentrant mutex — called from the main thread,
///   the same thread locks twice, and from another thread the caller waits for the job while the job waits for the lock).
/// - Calling from the main thread is not a deadlock — tauri-runtime-wry 2.11.3 `send_user_message` runs a job passed from the main thread
///   on the spot. So the real one receives the answer through a buffered channel (`mpsc::channel()` · `sync_channel(1)`) — with a zero-capacity channel, the send of a job that runs on that
///   spot waits forever for a receive that has not come yet.
/// - The real one waits with `recv_timeout` — if the main thread stalls, it does not hold the calling thread, and after the deadline `apply` = `Err` ·
///   `show_all` = false · `hide_all` = ends as a no-op.
// ADR-0302 — window state command
pub trait WindowControl: Send + Sync {
    /// Runs [`run_action`] as one main-thread job. `Err` = that message (bus `CONFLICT`).
    fn apply(&self, label: &str, action: WindowAction) -> Result<Applied, String>;
    /// Shows all windows — real = one show of `tray::actions` (the `how` branch). Answer = after that, main is the OS foreground window.
    fn show_all(&self, how: Activation) -> bool;
    /// Hides all windows — real = `tray::actions::hide_main_ui` (ADR-0229 — the same path as main's close · tray hide).
    fn hide_all(&self);
    /// Snapshot for `window.list` — `None` = no OS window for that label.
    fn state(&self, label: &str) -> Option<OsWindowState>;
    /// Is it a window outside the model — a `label` not in the model whose popout OS window is open now and is not a window of the prewarmed window pool (TRD
    /// S26-window-chrome §6-2 「모델 밖 창」 — an old popout on the boot failure screen, etc.). The service checks first whether it is in the model. Default =
    /// false (does not know windows outside the model).
    fn is_unmodeled(&self, _label: &str) -> bool {
        false
    }
    /// Runs [`run_unmodeled_action`] on a window outside the model as one main-thread job — same as `apply` but does not touch the model. `Err` =
    /// that message. Default = an `Err` saying there is no OS window.
    fn apply_unmodeled(&self, label: &str, _action: WindowAction) -> Result<Applied, String> {
        Err(no_os_window(label))
    }
}

/// Applies `action` to window `label` — read → [`plan`] → apply. This is the body that the real [`WindowControl::apply`] calls inside one main-thread
/// job (TRD §5-2).
///
/// - `Err` = no OS window · rejected because it is minimizing a non-visible window (the message of [`Unseen`]) · the OS call failed · the window is not in the model
///   or the lock is poisoned so the maximize marker could not be written (the deferred maximize table stays as it was then).
/// - Writing · withdrawing a desire writes the model marker first and then changes the table. That write takes the `layout` lock briefly, so ★the caller does not
///   hold that lock★.
pub fn run_action<O: WindowOps + ?Sized>(
    ops: &O,
    layout: &LayoutState,
    label: &str,
    action: WindowAction,
) -> Result<Applied, String> {
    run(ops, Some(layout), label, action)
}

/// Applies `action` to window `label` outside the model ([`WindowControl::is_unmodeled`]) — the same read → [`plan`] → apply as [`run_action`],
/// but ★it does not write the model's maximize marker★ (that window is not in the model · the deferred maximize table belongs to the OS window, so it is used as is). Bringing to the front is
/// not accepted (`Err` — the service does not send that action down this path · for a hidden window it is an action that goes all the way to showing all windows).
/// This is the body that the real [`WindowControl::apply_unmodeled`] calls inside one main-thread job (TRD S26-window-chrome §6-2).
pub fn run_unmodeled_action<O: WindowOps + ?Sized>(
    ops: &O,
    label: &str,
    action: WindowAction,
) -> Result<Applied, String> {
    if action == WindowAction::Raise {
        return Err(format!(
            "window 없음: {label} — 모델 밖 창은 앞으로 내지 않는다"
        ));
    }
    run(ops, None, label, action)
}

// `layout` = the model to write the maximize marker to — `None` = a window outside the model, so it does not write.
fn run<O: WindowOps + ?Sized>(
    ops: &O,
    layout: Option<&LayoutState>,
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
            if let Some(layout) = layout {
                set_model_maximized(layout, label, maximized)?;
            }
            ops.defer_maximized(label, maximized);
            Ok(Applied {
                deferred: true,
                ..Applied::default()
            })
        }
        Plan::Settle(maximized) => {
            if let Some(layout) = layout {
                set_model_maximized(layout, label, maximized)?;
            }
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
            // It was placed on the first-paint gate while all windows were being shown — that gate shows it (invariant 5).
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
/// The one-window part of a show that does not steal focus (TRD §6-2 ⓐ–ⓒ) — the show-all-windows of [`Activation::NoSteal`] calls it per window.
/// A window waiting for first paint · a window with no OS window is not touched and gives `false`. After showing, applies the desire written down ([`WindowOps::apply_deferred`]).
/// `focus` = asks for ⓒ (focus is given only when our process holds the foreground now). If showing · restoring fails, it logs and gives
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

// ADR-0299 · TRD §6-2 ⓐ → ⓑ → ⓒ. ⓑ applies the desire after restoring — if you apply and then restore (`SW_RESTORE`), it returns to the pre-minimize size and the desire is
//   overwritten. A window waiting for first paint is not touched once more here even if the caller already filtered it (invariant 5).
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

// ── service ──────────────────────────────────────────────────────────────────

/// Why the window argument · action could not hold — the bus converts it to a code (`Arg` = the code of [`WindowArgError`] · `NotApplied` = `CONFLICT`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WindowCtlError {
    #[error(transparent)]
    Arg(#[from] WindowArgError),
    #[error("{0}")]
    NotApplied(String),
}

/// Resolves window argument `window` (TRD §4-5) and applies `action` to that window. The argument is resolved inside the layout lock and the port is called after releasing it — if the
/// window closes in between and there is no OS window, the port rejects. If an argument not in the model is the label of a window outside the model ([`unmodeled`]), minimize · maximize ·
/// restore size are applied directly to that OS window ([`WindowControl::apply_unmodeled`] — the model is not changed). Bringing to the front is still
/// 「창 없음」 (no such window).
pub fn act(
    state: &LayoutState,
    control: &dyn WindowControl,
    window: &str,
    action: WindowAction,
) -> Result<Applied, WindowCtlError> {
    let label = match resolve(state, window) {
        Ok(label) => label,
        Err(error) => {
            return match unmodeled(control, &error) {
                Some(label) if action != WindowAction::Raise => control
                    .apply_unmodeled(&label, action)
                    .map_err(WindowCtlError::NotApplied),
                _ => Err(error),
            };
        }
    };
    control
        .apply(&label, action)
        .map_err(WindowCtlError::NotApplied)
}

// Window outside the model (TRD S26-window-chrome §6-2 · main decision M34) — the rejection that the window argument is not in the model (`NotFound`), and if that exact spelling is
//   the label of a window outside the model, that label. The command that the window buttons of the boot failure screen (`list_tabs` failure — an old popout not in the model, etc.) send with that window's
//   label lives here. The port is asked outside the layout lock (after `resolve` released it).
fn unmodeled(control: &dyn WindowControl, error: &WindowCtlError) -> Option<String> {
    match error {
        WindowCtlError::Arg(WindowArgError::NotFound(arg)) if control.is_unmodeled(arg) => {
            Some(arg.clone())
        }
        _ => None,
    }
}

/// Outcome of `window.close`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseOutcome {
    /// The popout was closed.
    Closed,
    /// It is main, so it was not closed and all windows were hidden.
    HiddenAll,
    /// Close if empty — that window has tabs or is already closed, so it was left as is (not an error).
    Kept,
}

/// Closes a window (TRD §5-4) — order: ① an argument of digits only is `NumberGiven` (even with `if_empty`) ② with `if_empty`, [`close_if_empty`] (argument =
/// persistent id · does not go through the resolver — that window is usually already not in the model) ③ resolve the argument; if main, hide all windows; if a popout,
/// `apply::close_window`. If an argument not in the model is the label of a window outside the model, [`close_unmodeled`] closes that window and gives `Closed`.
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
    let label = match resolve(state, window) {
        Ok(label) => label,
        Err(error) => {
            let Some(label) = unmodeled(control, &error) else {
                return Err(error);
            };
            return close_unmodeled(state, subs, host, &label);
        }
    };
    if label == MAIN_WINDOW_LABEL {
        // ADR-0229
        control.hide_all();
        return Ok(CloseOutcome::HiddenAll);
    }
    apply::close_window(state, subs, host, &label).map_err(WindowCtlError::NotApplied)?;
    Ok(CloseOutcome::Closed)
}

// Closing a window outside the model (TRD S26-window-chrome §6-2 「모델 밖 창」) — the model decision is redone inside one layout lock (the port
//   was asked outside the lock — in between, the commit of a new-window move may have put this window into the model). If it is in, the model part of closing a model window; if not,
//   a tombstone is set on the label (`ViewManager::mark_closed` — the tombstone that `commands::popout::drop_window_in_model` of a popout close request sets), then the lock is released and the OS window is withdrawn
//   (`host.close` — inside the lock it would deadlock · `WindowHost` header).
//   ★Do not defer the tombstone until after `host.close` or leave it to the `Destroyed` cleanup★ — destroy waits in the event loop, so if in between
//   a slot detach · the commit of dropping into a new window (`attach_view_as_new_window` · `dock`) attaches with this label, the later `Destroyed` cleanup deletes the window along with the tab that was moved
//   in. If the tombstone is set first, that commit is rejected with `WindowClosed` and goes through rollback, and the tab stays at the source.
// ADR-0302 — window state command
fn close_unmodeled(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    host: &dyn WindowHost,
    label: &str,
) -> Result<CloseOutcome, WindowCtlError> {
    {
        let mut mgr = state
            .0
            .lock()
            .map_err(|e| WindowCtlError::NotApplied(e.to_string()))?;
        if mgr.windows.contains_key(label) {
            apply::close_window_in_model(&mut mgr, subs, label)
                .map_err(WindowCtlError::NotApplied)?;
        } else {
            mgr.mark_closed(label);
        }
    } // ← lock dropped
    host.close(label);
    Ok(CloseOutcome::Closed)
}

/// Self-close at 0 tabs (TRD §5-5) — `Kept` if the window of persistent id `window_id` is in the model (that window has at least 1 tab), if it is not in the model and is a closing
/// window whose OS window is open, closes it and gives `Closed`; `Kept` if neither. The model decision is inside one lock, so there is no gap for a tab to come in in between
/// (if one comes in, it is the first branch). It picks by persistent id only, so it does not touch other windows. `Err` = lock poisoned.
// ADR-0302 — window state command
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

/// One row of `window.list` (TRD §6-3) — `window` = label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedWindow {
    pub window_id: String,
    pub window: WindowLabel,
    pub number: u32,
    pub main: bool,
    pub os: OsWindowState,
}

/// All windows in the model — in number order. Model rows are taken inside the lock and the OS state is read outside the lock and paired (a snapshot). A row with no OS window has all state
/// false. A closing window is not in the model, so it is not included. `Err` = lock poisoned.
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
    use crate::layout::apply::{
        DockDest, DockRequest, LabelSource, LayoutEvents, MoveFailed, SpareClaim, WindowPlace,
        WindowTabsPayload,
    };
    use crate::layout::manager::{DockError, DockMove, FocusPolicy, LayoutError, ViewManager};
    use crate::layout::tree;
    use crate::layout::types::{SlotContent, ViewSnapshot};

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

    // ── plan (§5-2 table) ──

    #[test]
    fn plan_follows_the_table_for_every_kind_of_window_and_action() {
        use WindowAction::{Maximize, Minimize, Raise, Unmaximize};
        let visible = |min, max| os(true, min, max, false);
        let hidden = |min, max| os(false, min, max, false);
        let awaiting = |min, max| os(false, min, max, true);
        let awaiting_refused = Plan::Refuse(Unseen::AwaitingFirstPaint);
        let hidden_refused = Plan::Refuse(Unseen::Hidden);
        // (action, state (minimized, maximized), decision) — 27 rows in total. A minimized window reads maximized as false (`OsWindowState`) — the state where both are
        // true is not listed.
        let rows = [
            // visible
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
            // waiting for first paint
            (Minimize, awaiting(false, false), awaiting_refused),
            (Maximize, awaiting(false, false), Plan::Defer(true)),
            (Maximize, awaiting(false, true), Plan::Settle(true)),
            (Unmaximize, awaiting(false, false), Plan::Settle(false)),
            (Unmaximize, awaiting(false, true), Plan::Defer(false)),
            (Raise, awaiting(false, false), Plan::Pending),
            // hidden (hide all · --hidden)
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

    // A window placed on the first-paint gate is a waiting window even if the visible flag is set — what shows that window is that gate.
    #[test]
    fn plan_treats_a_held_window_as_awaiting_even_if_it_reads_visible() {
        let held = os(true, false, false, true);
        assert_eq!(
            plan(WindowAction::Minimize, held),
            Plan::Refuse(Unseen::AwaitingFirstPaint)
        );
        assert_eq!(plan(WindowAction::Raise, held), Plan::Pending);
    }

    // ── imitating the main-thread queue (§5-2 「시험」) ──

    #[derive(Debug, Clone, Copy, Default)]
    struct FakeWindow {
        visible: bool,
        minimized: bool,
        /// It was placed maximized — it stays even while minimized and comes back maximized when restored. Reading gives false while minimized, as tao does.
        maximized: bool,
        held: bool,
    }

    /// OS stand-in holding windows · the deferred maximize table · the foreground. It blows up if OS minimize · maximize · restore comes to a hidden window or a waiting window is shown — with the real thing,
    /// that is where the window would be revealed.
    #[derive(Default)]
    struct FakeOps {
        windows: RefCell<BTreeMap<String, FakeWindow>>,
        deferred: RefCell<BTreeMap<String, bool>>,
        /// OS foreground window — `None` = another app.
        foreground: RefCell<Option<String>>,
        /// Show-all-windows first places this window on the first-paint gate again.
        hold_on_show_all: RefCell<Option<String>>,
        /// Applying the desire fails — the desire stays.
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

        // First-paint show (`state::placement::show_placed`) — releases the wait, shows, then applies and withdraws the desire written down.
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

        // Same as the tray order — popouts first, main last, and only main asks for focus.
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

    /// One main-thread queue — runs one at a time in the order added.
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

        /// Runs until the queue is empty — answer = the answer of the `Act` job (in the order added).
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

    // (a) First-paint show first — maximize is decided as a shown window and is applied on the spot, and the deferral table is empty.
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

    // (b) Maximize first — written down as a desire (the model marker too), and the later first-paint show applies and withdraws it.
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

    // A waiting window built maximized (a desire stands) — if restore size comes first, only the desire is withdrawn and it shows at normal size; if the show comes first, it is restored via the OS.
    // In both orders the OS window is already that size, so there is no deferral.
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

    // A hidden window — minimize is rejected, maximize is deferred, and bringing to the front first does a show-all-windows that does not steal focus, and that show
    // applies the deferred maximize. The foreground is held by our window (POP2) — so it asks for focus.
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

    // A window maximized, then minimized and hidden — even if maximize reads as false, restore size is written down as a deferral, and the show applies that desire after restoring (it comes back maximized)
    // and ends at normal size.
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

    // If the opposite desire was written first on a non-visible window, after the OS window is already that size the later action withdraws that desire — if left, the later
    // action is reversed when it is shown.
    #[test]
    fn a_later_action_on_a_hidden_window_overrides_an_earlier_wish() {
        // (OS maximize of a hidden window, earlier action, later action) — maximize of the window shown at the end = the later action
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

    // A desire left on a visible window (it stayed because applying failed) is withdrawn even by an action whose size is already that — if left, the next hide · show would apply it
    // and reverse that action.
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

    // After the restore-size desire of a hidden maximized window could not be applied at show and stayed, the maximize of the visible window (already maximized — no-op) withdraws that
    // desire. Even if it is hidden and shown again after that, it is not reversed to restore size.
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

    // If that window is placed on the first-paint gate while all windows are being shown, it is not revealed — that gate shows it.
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

    // Even bringing a hidden window to the front only shows it and does not ask for focus if another app is in the foreground — the answer is `focused: false`, as it is in fact (AC9).
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

    // The LLM's show-all-windows skips a window waiting for first paint — the first-paint gate will show that window soon.
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

    // If another app is in the foreground, it is only raised to the top and does not ask for focus — the answer is `focused: false`, as it is in fact (AC9).
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

    // Window outside the model (an old popout on the boot failure screen) — if visible, applied directly to the OS window; if already that size, only the desire is withdrawn; if hidden,
    //   the desire is written down. No branch touches the model (it was an empty model, so trying to write would have been `Err`).
    #[test]
    fn an_unmodeled_window_gets_the_os_action_without_the_model() {
        let ops = FakeOps::with(&[(MAIN_WINDOW_LABEL, SHOWN), (POP, SHOWN), (POP2, HIDDEN)]);
        ops.deferred.borrow_mut().insert(POP.to_string(), false);

        assert_eq!(
            run_unmodeled_action(&ops, POP, WindowAction::Maximize),
            Ok(Applied::default())
        );
        assert!(ops.window(POP).maximized);
        assert_eq!(
            run_unmodeled_action(&ops, POP, WindowAction::Maximize),
            Ok(Applied::default()),
            "이미 최대화 — 바람만 거둔다"
        );
        assert_eq!(
            run_unmodeled_action(&ops, POP, WindowAction::Unmaximize),
            Ok(Applied::default())
        );
        assert_eq!(
            run_unmodeled_action(&ops, POP, WindowAction::Minimize),
            Ok(Applied::default())
        );
        assert!(ops.window(POP).minimized && !ops.window(POP).maximized);
        assert_eq!(
            ops.calls(),
            [
                format!("maximize:{POP}"),
                format!("drop_deferred:{POP}"),
                format!("drop_deferred:{POP}"),
                format!("unmaximize:{POP}"),
                format!("drop_deferred:{POP}"),
                format!("minimize:{POP}"),
            ]
        );

        assert_eq!(
            run_unmodeled_action(&ops, POP2, WindowAction::Maximize),
            Ok(Applied {
                deferred: true,
                ..Applied::default()
            }),
            "숨은 창은 OS 창을 건드리지 않고 바람을 적는다"
        );
        assert_eq!(ops.deferred(), [(POP2.to_string(), true)]);
        assert!(!ops.window(POP2).visible);
    }

    #[test]
    fn an_unmodeled_window_is_not_raised_and_one_without_an_os_window_fails() {
        let ops = FakeOps::with(&[(MAIN_WINDOW_LABEL, SHOWN), (POP, HIDDEN)]);

        let err = run_unmodeled_action(&ops, POP, WindowAction::Raise).unwrap_err();
        assert!(err.contains(POP), "{err}");
        assert!(ops.calls().is_empty(), "{:?}", ops.calls());
        let err = run_unmodeled_action(&ops, POP2, WindowAction::Minimize).unwrap_err();
        assert!(err.contains(POP2), "{err}");
    }

    // If the model marker cannot be written, the desire is neither written nor withdrawn — the table and the model do not diverge.
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

    // ── service (window argument · port outside the lock) ──

    /// Recording window state port — on every call it checks whether it is outside the layout lock. `unmodeled` = the label to answer as a window outside the model · `admits` = the label that enters the model right after
    /// it answered that it is a window outside the model (the commit of a new-window move lands in that gap — once).
    struct Control {
        layout: LayoutState,
        calls: Mutex<Vec<String>>,
        states: Mutex<BTreeMap<String, OsWindowState>>,
        unmodeled: Mutex<BTreeSet<String>>,
        admits: Mutex<BTreeSet<String>>,
    }

    impl Control {
        fn new(layout: &LayoutState) -> Self {
            Control {
                layout: layout.clone(),
                calls: Mutex::new(Vec::new()),
                states: Mutex::new(BTreeMap::new()),
                unmodeled: Mutex::new(BTreeSet::new()),
                admits: Mutex::new(BTreeSet::new()),
            }
        }

        fn unmodeled(&self, label: &str) {
            self.unmodeled.lock().unwrap().insert(label.to_string());
        }

        fn admits(&self, label: &str) {
            self.admits.lock().unwrap().insert(label.to_string());
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

        fn is_unmodeled(&self, label: &str) -> bool {
            self.outside(format!("is_unmodeled:{label}"));
            let unmodeled = self.unmodeled.lock().unwrap().contains(label);
            if self.admits.lock().unwrap().remove(label) {
                self.layout.0.lock().unwrap().create_window(label).unwrap();
            }
            unmodeled
        }

        fn apply_unmodeled(&self, label: &str, action: WindowAction) -> Result<Applied, String> {
            self.outside(format!("apply_unmodeled:{label}:{action:?}"));
            Ok(Applied::default())
        }
    }

    /// Window port stand-in — only windows in `opened` are open (main is always open).
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
            control.calls()[2..],
            ["is_unmodeled:slot-popup-9".to_string()],
            "거절된 인자는 동작 포트까지 가지 않는다 — 모델에 없는 인자만 모델 밖 창인지 묻는다"
        );
    }

    // Window buttons of a window outside the model (an old popout on the boot failure screen) — minimize · maximize · restore size go by that label to the outside-model action,
    //   and bringing to the front is still 「창 없음」 (no such window). The model is unchanged.
    #[test]
    fn act_on_an_unmodeled_window_reaches_its_os_window_outside_the_lock() {
        let layout = layout_with_popouts(&[]);
        let control = Control::new(&layout);
        control.unmodeled(POP);

        for action in [
            WindowAction::Minimize,
            WindowAction::Maximize,
            WindowAction::Unmaximize,
        ] {
            assert_eq!(
                act(&layout, &control, &format!(" {POP} "), action),
                Ok(Applied::default())
            );
        }
        assert_eq!(
            act(&layout, &control, POP, WindowAction::Raise),
            Err(WindowCtlError::Arg(WindowArgError::NotFound(
                POP.to_string()
            )))
        );
        assert_eq!(
            act(&layout, &control, POP2, WindowAction::Minimize),
            Err(WindowCtlError::Arg(WindowArgError::NotFound(
                POP2.to_string()
            ))),
            "OS 창이 없는(모델 밖 창이 아닌) label 은 그대로 「창 없음」"
        );
        let calls = control.calls();
        assert_eq!(
            calls
                .iter()
                .filter(|call| call.starts_with("apply"))
                .cloned()
                .collect::<Vec<_>>(),
            [
                format!("apply_unmodeled:{POP}:Minimize"),
                format!("apply_unmodeled:{POP}:Maximize"),
                format!("apply_unmodeled:{POP}:Unmaximize"),
            ]
        );
        assert_eq!(
            layout.0.lock().unwrap().windows.len(),
            1,
            "모델은 main 하나 그대로"
        );
    }

    // Closing a window outside the model — set a tombstone on the label, withdraw that OS window, then `Closed` (nothing to change in the model's window entries). A label that is not
    //   a window outside the model is still 「창 없음」 (no such window).
    #[test]
    fn closing_an_unmodeled_window_closes_its_os_window() {
        let layout = layout_with_popouts(&[]);
        let (control, host) = (Control::new(&layout), Host::new(&layout));
        control.unmodeled(POP);
        host.open_os(POP);

        assert_eq!(
            close(&layout, &Subs, &host, &control, POP, false),
            Ok(CloseOutcome::Closed)
        );
        assert_eq!(host.closed(), [POP]);
        assert_eq!(
            layout.0.lock().unwrap().create_window(POP),
            Err(LayoutError::WindowClosed(POP.to_string())),
            "그 label 로는 모델 창이 서지 않는다"
        );
        assert_eq!(
            close(&layout, &Subs, &host, &control, POP2, false),
            Err(WindowCtlError::Arg(WindowArgError::NotFound(
                POP2.to_string()
            )))
        );
        assert_eq!(host.closed(), [POP]);
        assert!(
            !control.calls().iter().any(|call| call.starts_with("apply")),
            "{:?}",
            control.calls()
        );
        layout.0.lock().unwrap().assert_number_invariants();
    }

    /// A window port where, right after a new window is opened · before it is put into the model (phase B), that window's web close button sends `window.close <label>` —
    /// that window is still a window outside the model (boot failure screen). `close` only queues the destroy, so its `Destroyed` cleanup does not come inside this test —
    /// the commit stands before that cleanup.
    struct ClosesOnOpen<'a> {
        host: Host,
        control: &'a Control,
    }

    impl WindowHost for ClosesOnOpen<'_> {
        fn open(&self, label: &str) -> Result<(), String> {
            self.host.open_os(label);
            assert_eq!(
                close(&self.host.layout, &Subs, self, self.control, label, false),
                Ok(CloseOutcome::Closed)
            );
            Ok(())
        }

        fn open_at(&self, label: &str, _at: &WindowPlace, _activate: bool) -> Result<(), String> {
            self.open(label)
        }

        // The tombstone stands before the OS window is withdrawn — a commit that attaches while destroy is queued is also rejected.
        fn close(&self, label: &str) {
            assert_eq!(
                self.host.layout.0.lock().unwrap().create_window(label),
                Err(LayoutError::WindowClosed(label.to_string()))
            );
            self.host.close(label);
        }

        fn is_open(&self, label: &str) -> bool {
            self.host.is_open(label)
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

    struct NextIsPop;

    impl LabelSource for NextIsPop {
        fn next_label(&self) -> String {
            POP.to_string()
        }
    }

    struct NoEvents;

    impl LayoutEvents for NoEvents {
        fn layout_updated(&self, _snapshot: &ViewSnapshot) {}
        fn window_tabs_updated(&self, _tabs: &WindowTabsPayload) {}
    }

    // The close of a window outside the model gets caught in the gap of a new-window move — the tombstone stands before the OS window is withdrawn, so the commit of slot detach · dropping into a new window is
    //   rejected with `WindowClosed`, and the slot · tab that was to be moved stays at the source (if the tombstone were left to the `Destroyed` cleanup, the commit would stand and that cleanup would delete the
    //   tab along with the moved one · TRD S26-window-chrome §6-2 「모델 밖 창」).
    #[test]
    fn closing_an_unmodeled_window_refuses_a_move_into_it_and_keeps_the_source() {
        let layout = layout_with_popouts(&[]);
        let control = Control::new(&layout);
        control.unmodeled(POP);
        let host = ClosesOnOpen {
            host: Host::new(&layout),
            control: &control,
        };
        let (main_tab, slot, content) = {
            let mut mgr = layout.0.lock().unwrap();
            let main_tab = mgr.windows[MAIN_WINDOW_LABEL].active;
            let slot = tree::first_slot_id(&mgr.views[&main_tab].layout);
            mgr.set_slot_content(main_tab, slot, SlotContent::AgentList)
                .unwrap();
            (main_tab, slot, SlotContent::AgentList)
        };

        let err = apply::move_slot_to_window(
            &layout, &Subs, &NoEvents, &host, &NextIsPop, main_tab, slot, None,
        )
        .unwrap_err();
        assert!(
            err.ends_with(&LayoutError::WindowClosed(POP.to_string()).to_string()),
            "{err}"
        );
        {
            let mgr = layout.0.lock().unwrap();
            assert_eq!(mgr.slot_content(main_tab, slot), Ok(content));
            assert!(!mgr.windows.contains_key(POP));
        }
        assert_eq!(host.host.closed(), [POP, POP], "닫기 단추 · 연 창 거두기");

        let docked = layout
            .0
            .lock()
            .unwrap()
            .create_tab(MAIN_WINDOW_LABEL, None)
            .unwrap();
        let tabs_before = layout.0.lock().unwrap().windows[MAIN_WINDOW_LABEL]
            .tabs
            .clone();
        let refused = apply::dock_tab(
            &layout,
            &Subs,
            &NoEvents,
            &host,
            &NextIsPop,
            DockRequest {
                view: docked,
                to: DockDest::NewWindow {
                    at: WindowPlace {
                        pos: None,
                        size: None,
                    },
                },
                focus: FocusPolicy::Reveal,
                mv: DockMove::default(),
            },
        );
        assert_eq!(
            refused.map(|_| ()),
            Err(DockError::WindowClosed(POP.to_string()))
        );
        let mgr = layout.0.lock().unwrap();
        assert_eq!(mgr.windows[MAIN_WINDOW_LABEL].tabs, tabs_before);
        assert!(!mgr.windows.contains_key(POP));
        mgr.assert_number_invariants();
    }

    // After the port answered that it is a window outside the model · before the layout lock is taken, that window entered the model (the commit of a new-window move) — it goes to closing a model window:
    //   delete from the model and withdraw the OS window while holding the number until its `Destroyed`.
    #[test]
    fn an_unmodeled_window_that_entered_the_model_is_closed_as_a_model_window() {
        let layout = layout_with_popouts(&[]);
        let (control, host) = (Control::new(&layout), Host::new(&layout));
        control.unmodeled(POP);
        control.admits(POP);
        host.open_os(POP);

        assert_eq!(
            close(&layout, &Subs, &host, &control, POP, false),
            Ok(CloseOutcome::Closed)
        );
        assert_eq!(host.closed(), [POP]);
        let mut mgr = layout.0.lock().unwrap();
        assert!(!mgr.windows.contains_key(POP));
        mgr.assert_number_invariants();
        assert!(mgr.release_closing(POP), "번호는 그 `Destroyed` 까지 쥔다");
    }

    // ── close join (§5-4) · close if empty (§5-5) ──

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

    // 「모델에서만 빠지고 OS 창은 남은 창」 (a window removed only from the model while its OS window remains) — the state of a popout whose tabs became 0. Only that window is closed by persistent id, and windows in the model · other ids ·
    // windows whose OS window is already gone are left as they are (AC24 · AC32).
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

    // ── window list (§6-3) ──

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
