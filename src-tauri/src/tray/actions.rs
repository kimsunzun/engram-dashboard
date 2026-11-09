//! tray actions — side-effect functions that the tray handler and the Tauri command **share** (impure: depends on Tauri/discovery).
//!
//! ## Why this module is separate (CLAUDE.md 「LLM-우선 제어」 — load-bearing)
//! The native tray popup is not a WebView DOM, so cdp cannot click it directly. So a tray menu click and
//! an LLM/cdp invoke are made to call the **same internal function** (here), so the two operating parties share the same behavior.
//! Both the tray handler (mod.rs on_menu_event) and the command (commands/tray.rs) call only these functions —
//! no duplicating behavior logic. Separate from core.rs (pure decisions): this one touches real windows/icons/the daemon (impure).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::menu::CheckMenuItem;
use tauri::{AppHandle, Emitter, Manager, Wry};

use tauri_plugin_autostart::ManagerExt;

use super::core::{self, IconState};
use super::TrayIcons;
use crate::commands::popout::is_popup_label;
use crate::commands::spare::is_spare_window;
use crate::commands::window_ctl::TauriOps;
use crate::daemon_client::shutdown::{StopOrigin, StopOutcome, StopPlan};
use crate::daemon_client::DaemonClient;
use crate::layout::window_ctl::{self, Activation, WindowOps};
use crate::layout::{LayoutState, MAIN_WINDOW_LABEL};
use crate::spare::pool::TrayShow;
use crate::spare::SpareState;

// The tray icon id (given to the builder, looked up again with tray_by_id). A single tray, so a fixed string.
pub const TRAY_ID: &str = "engram-main-tray";

// The single-owner state of the daemon liveness push (ADR-0028 single channel). Every source (observer probe · turn-on · turn-off) enters through
// [`publish_daemon_liveness`] and is compared with `last` here → tray + emit only on change.
//
// ★Why unify into manage state (M-1 — load-bearing)★: previously the observer (periodic probe) and the turn-off immediate confirmation
// (StopOutcome::DaemonClosed → set gray directly) **did not share state**, so there was a race — after clicking turn-off,
// in the death-window while the daemon was still dying (between connection closed and process exit), the observer probed alive=true
// in that window and turned the icon just set gray back to color (S13 race recurred). Now all liveness
// sources converge on this single entry point, and turn-off sets a suppression window with [`force_daemon_down`] to close that race.
#[derive(Default)]
pub struct LivenessState {
    // The alive value pushed last (None = never pushed yet). The basis for change detection.
    last: Mutex<Option<bool>>,
    // None = no suppression. Some(t) = ignore alive=true probes while now<t.
    suppress_alive_until: Mutex<Option<Instant>>,
}

// The death-window suppression time right after turn-off. The margin until the daemon closes the connection and actually exits — set to the same 5 seconds as discovery's
// stop polling timeout, so that even if a probe reports a false alive within it, it is ignored.
const STOP_GRACE: Duration = Duration::from_secs(5);

// Keeps the handle of the "부팅 시 자동 시작" (start automatically at boot) CheckMenuItem (ADR-0027 §55).
//
// ★Why hold the handle in manage (load-bearing)★: after a toggle the check mark must be updated at once with set_checked, but
// without the CheckMenuItem handle there is no update target. Re-looking it up by tray.menu() → id needs a cumbersome MenuItem
// downcast (CheckMenuItem), so the handle build_tray made is kept as state as is and set_checked is called on it directly.
pub struct AutostartCheck(pub CheckMenuItem<Wry>);

// Bring back all of main and the popout windows — hidden windows are shown and minimized windows are unminimized. `main` remains in the name because the command of the same name
// is the LLM control surface — changing the name changes the surface. There are two branches (TRD S26-window-chrome §6-2 — which is which
// branch is in the `Activation` docs):
// - Click (`TrayClick`) — even windows that were already up are raised above other apps, and focus goes last to main, so main is on top and the popouts come right
//   below it. A window that was waiting for first paint also has that wait withdrawn and is shown (it is a show a person pressed — §2-4 invariant 5).
// - Not stealing focus (`NoSteal`) — [`show_main_ui_no_steal`].
// ★`NoSteal` is called only from the main thread★ — that branch calls a raw Win32 raise (`SetWindowPos` inside `raise_without_activation`).
//   Bus · invoke come in as main-thread work of `commands::window_ctl::TauriWindowControl`.
// Even if one window fails (a popout closed after the list was taken, etc.), the rest and main keep being processed — do not cut it off with `?`.
// Windows of the prewarmed-window pool are not shown in either branch — only a window that left the pool by adoption in the meantime is shown like a normal window ([`show_ui_windows`]).
// ADR-0229 · ADR-0299 · ADR-0302
pub(crate) fn show_main_ui(app: &AppHandle, how: Activation) {
    match how {
        Activation::TrayClick => show_main_ui_on_click(app),
        Activation::NoSteal => show_main_ui_no_steal(app, &TauriOps::new(app)),
    }
}

// ★The order show()→unminimize()→apply the desire→set_focus() per window is load-bearing★:
// - show → unminimize → set_focus (Windows focus-stealing) — in any other order, from hidden/minimized you may get only a taskbar flash
//   and the window may not actually come up (TRD §4).
// - Applying the desire comes after unminimize — put before it, a minimized window loses its original-size desire (`apply_deferred_maximize` header ·
//   TRD S26-window-chrome §6-2).
// ★The order between windows (main last) is also load-bearing★: all four activate the window (tao 0.35 Windows = `SW_SHOW` ·
//   `SW_RESTORE` · `SW_MAXIMIZE`/`SW_RESTORE` of applying the desire · `SetForegroundWindow`), so if anything is done after main, that window takes
//   focus. The order is decided by
//   [`core::ui_windows`].
// ADR-0229 · ADR-0299
fn show_main_ui_on_click(app: &AppHandle) {
    let windows = app.webview_windows();
    let show = |label: &str| {
        let Some(w) = windows.get(label) else {
            return false;
        };
        // If the window was waiting for first paint, withdraw that raise — this show ends that wait (the same path as hide). If left,
        //   a notification · time limit that comes later would show the already-shown window once more outside this order (`crate::first_paint`).
        crate::first_paint::cancel_hold(app, label);
        let _ = w.show();
        let _ = w.unminimize();
        crate::state::placement::apply_deferred_maximize(&w.as_ref().window());
        let _ = w.set_focus();
        true
    };
    with_usage_visibility(app, |usage| {
        show_ui_windows(
            windows.keys().map(String::as_str),
            usage,
            |label| is_spare_window(app, label),
            |label| tray_show(app, label),
            show,
        );
    });
}

/// Show all windows without stealing focus (`Activation::NoSteal` — TRD S26-window-chrome §6-2) — in the same window order as the click branch
/// (skip prewarmed windows · prewarmed windows that left the pool first · main last), it is `window_ctl::show_one_no_steal` per window: windows waiting
/// for first paint are skipped (§2-4 invariant 5 — that window's first-paint gate will show it soon), and it raises to the top without activation, shows · unminimizes · applies the recorded
/// maximize desire. Focus is given only to main, and only when our process already holds the foreground.
///
/// ★Call it from the main thread★ — `ops` calls a raw Win32 raise, and the per-window wait decision · show must not
/// interleave with the first-paint show (§5-2). Bus · invoke call it as main-thread work of `commands::window_ctl::TauriWindowControl`.
// ADR-0299
pub(crate) fn show_main_ui_no_steal(app: &AppHandle, ops: &dyn WindowOps) {
    let windows = app.webview_windows();
    with_usage_visibility(app, |usage| {
        show_ui_windows_no_steal(
            windows.keys().map(String::as_str),
            usage,
            |label| is_spare_window(app, label),
            |label| tray_show(app, label),
            ops,
        );
    });
}

fn show_ui_windows_no_steal<'a>(
    labels: impl IntoIterator<Item = &'a str>,
    usage: &dyn UsageVisibility,
    spare: impl Fn(&str) -> bool,
    tray_show: impl Fn(&str) -> TrayShow,
    ops: &dyn WindowOps,
) {
    show_ui_windows(labels, usage, spare, tray_show, |label| {
        window_ctl::show_one_no_steal(ops, label, label == MAIN_WINDOW_LABEL)
    });
}

fn tray_show(app: &AppHandle, label: &str) -> TrayShow {
    app.try_state::<Arc<SpareState>>()
        .map_or(TrayShow::Untouched, |spare| spare.on_tray_show(label))
}

// The order of the tray show — run [`for_each_ui_window`] as a show, and for each label skipped by the prewarmed-window predicate (`spare`), ask the pool
// (`tray_show` — production = `SpareState::on_tray_show`). Only a window that left the pool by adoption after the predicate decision (`Adopted`) is shown with the same `show` as other windows,
// and a handed-over window (`Rehold` — the pool re-applies the raise so adoption ③ shows it) and a prewarmed window not handed over (`Untouched`) are
// not shown — showing them would reveal a frame with no content.
// ★`Adopted` is shown before the windows whose order was decided★ — if shown after, its focus steals main's last focus.
// ★`tray_show` must return after releasing the pool lock★ — right after, usage interest takes the layout lock (order = layout → pool) and calls the
//   window (no window call while holding the pool lock — `SpareState`).
// (TRD S21-slot-docking §3-9 「숨어 있기」)
// ADR-0299
fn show_ui_windows<'a>(
    labels: impl IntoIterator<Item = &'a str>,
    usage: &dyn UsageVisibility,
    spare: impl Fn(&str) -> bool,
    tray_show: impl Fn(&str) -> TrayShow,
    show: impl Fn(&'a str) -> bool,
) {
    for_each_ui_window(
        labels,
        true,
        usage,
        spare,
        |skipped| {
            for &label in skipped {
                if tray_show(label) == TrayShow::Adopted {
                    usage.set_visible(label, true);
                    show(label);
                }
            }
        },
        &show,
    );
}

// Hide all of main and the popout windows (not destruction — the WebView stays resident, and [`show_main_ui`] brings it back). Main's X also
// comes here. The targets are the same as [`show_main_ui`] — if show cannot bring a hidden window back, there is no way to get it back.
// ADR-0229 · ADR-0299
pub fn hide_main_ui(app: &AppHandle) {
    let windows = app.webview_windows();
    let labels: Vec<&str> = windows.keys().map(String::as_str).collect();
    with_usage_visibility(app, |usage| {
        hide_ui_windows(
            &labels,
            usage,
            |label| is_spare_window(app, label),
            |label| {
                let Some(w) = windows.get(label) else {
                    return false;
                };
                // If the window was waiting for first paint, withdraw that raise — a notification · time limit that comes after hiding would show that window again
                //   (`crate::first_paint`). It is withdrawn even if hiding fails — it is a window that was asked to hide.
                crate::first_paint::cancel_hold(app, label);
                match w.hide() {
                    Ok(()) => true,
                    Err(e) => {
                        tracing::warn!(
                            label,
                            "[tray] 창 숨기기 실패 — 사용량 관심에서 빼지 않는다: {e}"
                        );
                        false
                    }
                }
            },
            |label| crate::first_paint::cancel_hold(app, label),
        );
    });
}

// The order of the tray hide — after running [`for_each_ui_window`] as a hide, withdraw the raise of all popout labels regardless of the prewarmed-window
// predicate (`cancel_hold` — a no-op for a label with no raise). That order does not call `hide` on windows skipped by the predicate (including a handed-over prewarmed window), so
// if the raise set at hand-over stays, adoption ③ would show that window over the hidden UI.
// ★Do not ask the pool again just to pick out the skipped ones★ — it races with adoption ① ([`for_each_ui_window`]). Put it after the order, so the raise of a window handed over
//   while the order was running is also withdrawn (TRD S21-slot-docking §3-9 「숨어 있기」).
// ADR-0299
fn hide_ui_windows<'a>(
    labels: &[&'a str],
    usage: &dyn UsageVisibility,
    spare: impl Fn(&str) -> bool,
    hide: impl FnMut(&'a str) -> bool,
    mut cancel_hold: impl FnMut(&'a str),
) {
    for_each_ui_window(labels.iter().copied(), false, usage, spare, |_| {}, hide);
    for &label in labels.iter().filter(|label| is_popup_label(label)) {
        cancel_hold(label);
    }
}

/// The spot that tells usage interest when tray show·hide changes one window's visibility (ADR-0012 seam — production =
/// [`LayoutUsageVisibility`], test = recording type). The usage slot of a hidden window drops out of interest (TRD S21 usage-limit-slot
/// §1-7 「숨김」).
pub(crate) trait UsageVisibility {
    fn set_visible(&self, label: &str, visible: bool);
}

/// Production implementation — per window, takes the ViewManager lock **briefly** and recomputes only usage interest inside it.
pub(crate) struct LayoutUsageVisibility<'a> {
    pub state: &'a LayoutState,
    pub client: &'a DaemonClient,
}

impl UsageVisibility for LayoutUsageVisibility<'_> {
    fn set_visible(&self, label: &str, visible: bool) {
        let Ok(mgr) = self.state.0.lock() else {
            tracing::warn!(
                label,
                visible,
                "[tray] 레이아웃 락 오염 — 사용량 관심 재계산 스킵"
            );
            return;
        };
        self.client.usage_window_visibility(label, visible, &mgr);
    }
}

// An assembly where client creation failed (`lib.rs` setup — the app keeps coming up) has no usage interest — it handles only windows.
struct NoUsageVisibility;

impl UsageVisibility for NoUsageVisibility {
    fn set_visible(&self, _label: &str, _visible: bool) {}
}

pub(crate) fn with_usage_visibility(app: &AppHandle, run: impl FnOnce(&dyn UsageVisibility)) {
    match (
        app.try_state::<LayoutState>(),
        app.try_state::<Arc<DaemonClient>>(),
    ) {
        (Some(state), Some(client)) => run(&LayoutUsageVisibility {
            state: &state,
            client: &client,
        }),
        _ => run(&NoUsageVisibility),
    }
}

// For each window the tray will handle ([`core::ui_windows`] order), handle the OS window (`os` — `true` = applied to that window) and tell usage interest
// the visibility. A window that is `spare` (a window of the spare-window pool — TRD S21-slot-docking §3-9) is not handled — that window waits
// hidden, so the show reveals it. A label skipped that way is received by `skipped` — exactly once, after filtering · before the first `os` call
// (even if nothing was skipped). ★`spare` is asked only once per popout label and that decision is passed to `skipped`★ — if the caller
// asks the pool again, that question races with adoption ① and a label handled by neither side can arise.
// ★The order differs by direction — toward the side that is safe even if wrong★:
// - Show = interest first · OS after. Even if the show fails, the only remaining cost is a lookup for a window that is not visible.
// - Hide = OS first · only hidden windows are removed from interest. If a failed hide were removed, the value of a **visible** window would stall.
// ★`os` is called outside the lock★ — the production implementation's ViewManager lock is taken and released within a single `set_visible`. This function
//   also runs off the main thread (`state::placement::TauriRestoreWindows::set_shown`), and putting OS window calls inside that lock carries the same
//   deadlock risk as the window-host port (`WindowHost` doc of `layout::apply`).
// ADR-0295 · ADR-0299
pub(crate) fn for_each_ui_window<'a>(
    labels: impl IntoIterator<Item = &'a str>,
    visible: bool,
    usage: &dyn UsageVisibility,
    spare: impl Fn(&str) -> bool,
    skipped: impl FnOnce(&[&'a str]),
    mut os: impl FnMut(&'a str) -> bool,
) {
    let (spares, rest): (Vec<&'a str>, Vec<&'a str>) = labels
        .into_iter()
        .partition(|&label| is_popup_label(label) && spare(label));
    skipped(&spares);
    for label in core::ui_windows(rest, is_popup_label) {
        if visible {
            usage.set_visible(label, true);
            os(label);
        } else if os(label) {
            usage.set_visible(label, false);
        }
    }
}

// Quit the whole app — stop the daemon on the one stop path's `Exit` plan, then exit whatever the outcome (ADR-0026).
//
// `Exit` waits at most 4 s on this thread and never kills (TRD S21 trd-3-2-u5 §3-3): the daemon is detached and outlives
// the app if the stop does not land (ADR-0024). The stop runs on the daemon client's runtime, which is the only runtime
// the shell owns — with no client (setup failed to build it) there is no stop, only the exit (R10).
pub fn quit_app(app: &AppHandle) {
    let client = app.try_state::<Arc<DaemonClient>>();
    stop_before_exit(
        client
            .as_ref()
            .map(|client| client.inner() as &dyn TrayDaemonClient),
    );
    app.exit(0);
}

/// What the tray's daemon on / off and quit ask of the daemon client — a seam so their order is tested without a
/// runtime or a daemon (ADR-0012). The real one is the `Arc<DaemonClient>` impl; each call blocks or fires as the
/// [`DaemonClient`] method it names.
pub(crate) trait TrayDaemonClient {
    /// [`DaemonClient::wait_stop_over_blocking`].
    fn wait_stop_over(&self);
    /// [`DaemonClient::stop_daemon_blocking`].
    fn stop(&self, plan: StopPlan, origin: StopOrigin) -> StopOutcome;
    /// [`DaemonClient::ensure_in_background`].
    fn attach_in_background(&self);
}

impl TrayDaemonClient for Arc<DaemonClient> {
    fn wait_stop_over(&self) {
        self.wait_stop_over_blocking();
    }

    fn stop(&self, plan: StopPlan, origin: StopOrigin) -> StopOutcome {
        self.stop_daemon_blocking(plan, origin)
    }

    fn attach_in_background(&self) {
        self.ensure_in_background();
    }
}

// The stop half of [`quit_app`] — `None` = no client, nothing stopped (R10).
fn stop_before_exit(client: Option<&dyn TrayDaemonClient>) -> Option<StopOutcome> {
    let Some(client) = client else {
        tracing::warn!("[tray] quit_app: no daemon client — exiting without stopping the daemon");
        return None;
    };
    let outcome = client.stop(StopPlan::Exit, StopOrigin::AppExit);
    tracing::info!(?outcome, "[tray] quit_app: daemon stop done — exiting");
    Some(outcome)
}

// Probe daemon alive and feed it into the single publish entry point ([`publish_daemon_liveness`]).
//
// ★Unified through publish (ADR-0028 single channel — load-bearing)★: it used to call set_icon directly here, but
// now it passes the probe result to publish, which handles **duplicate blocking (set only on change) · emit · suppression-window decision** in one place.
// That way all liveness sources — observer · turn-on · turn-off — pass through the same gate (LivenessState), and the M-1 race and
// the emit asymmetry (m-2) are gone.
//
// daemon_status is a daemon.json + PID liveness decision (fast, non-blocking level). Periodic detection of external/crash death
// is the observer's job (spawn_daemon_observer) — this is for updates right after an action · at the initial setup.
pub fn refresh_tray_icon(app: &AppHandle) {
    let data_dir = crate::discovery::default_data_dir();
    let alive = crate::discovery::daemon_status(&data_dir).alive;
    publish_daemon_liveness(app, alive);
}

// Daemon liveness single publish — only on change, tray set_icon + emit("daemon-status-changed").
//
// ★During the suppression window, alive=true is ignored (blocks the M-1 race — load-bearing)★: inside the suppression window (suppress_alive_until),
// alive=true is discarded. alive=false always passes through (turn-off confirmation — regardless of suppression).
//
// ★No set_icon/emit while holding the lock (ADR-0006 lock order)★: only the change decision · suppression decision are done inside the lock, and after
// dropping the lock set_icon/emit (external calls · main-thread post) are called. set_icon itself is guaranteed on the main thread by
// set_tray_icon_state with run_on_main_thread.
pub fn publish_daemon_liveness(app: &AppHandle, observed_alive: bool) {
    let st = app.state::<LivenessState>();

    // Suppression-window decision: an alive=true report inside the death-window may be false, so ignore it (take the lock briefly and drop).
    if observed_alive {
        if let Some(until) = *st.suppress_alive_until.lock().unwrap() {
            if Instant::now() < until {
                return; // death-window — ignore false alive
            }
        }
    }

    // Change decision (lock briefly) → drop → side effects. If no change, skip both set_icon/emit.
    {
        let mut last = st.last.lock().unwrap();
        if *last == Some(observed_alive) {
            return;
        }
        *last = Some(observed_alive);
    } // ← the last lock is dropped here. The side effects below run without holding the lock.

    set_tray_icon_state(app, core::icon_state_for(observed_alive));
    if let Err(e) = app.emit("daemon-status-changed", observed_alive) {
        tracing::warn!("[tray] daemon-status-changed emit 실패: {e}");
    }
    tracing::debug!(alive = observed_alive, "[tray] 데몬 생사 push");
}

// Turn-off confirmation (StopOutcome::DaemonClosed) — force gray and set the death-window suppression window.
//
// ★Suppression window first, publish(false) after (M-1 — load-bearing)★: set suppress_alive_until first, then
// confirm gray with publish(false). false passes regardless of the suppression decision (suppression
// blocks only alive=true), so gray+emit goes out immediately, and for STOP_GRACE after that, the observer's false alive=true probe is discarded by publish
// at the suppression window → the gray does not go back to color.
pub fn force_daemon_down(app: &AppHandle) {
    *app.state::<LivenessState>()
        .suppress_alive_until
        .lock()
        .unwrap() = Some(Instant::now() + STOP_GRACE);
    publish_daemon_liveness(app, false); // false passes regardless of suppression → gray+emit, last=false
}

// Directly replace the tray icon with the given [`IconState`] (**no PID probe**).
//
// ★The probe-bypass path (load-bearing — prevents recurrence of the S13 race)★: `refresh_tray_icon` *queries* the state with
// `daemon_status().alive` (PID probe = OpenProcess) and sets it, but this function sets the state the caller has
// already confirmed as is. So the StopOutcome::DaemonClosed (confirmed off) path does not go through the probe and sets gray directly with this function.
// General updates that need a probe use `refresh_tray_icon`.
//
// The set_icon main-thread guarantee is the same as `refresh_tray_icon` (run_on_main_thread). The two icon sets are
// cloned from TrayIcons state (cheap — internal Cow/Arc).
pub fn set_tray_icon_state(app: &AppHandle, state: IconState) {
    let icons = app.state::<TrayIcons>();
    // Image<'static> is an internal Cow — clone is cheap. Cloned in advance to move into the main-thread closure.
    let img = match state {
        IconState::Active => icons.active.clone(),
        IconState::Inactive => icons.inactive.clone(),
    };

    let app_main = app.clone();
    let set = move || {
        if let Some(tray) = app_main.tray_by_id(TRAY_ID) {
            if let Err(e) = tray.set_icon(Some(img)) {
                tracing::warn!("[tray] set_icon 실패: {e}");
            }
        }
    };
    // If already on the main thread, immediately; otherwise post to the main loop.
    if let Err(e) = app.run_on_main_thread(set) {
        tracing::warn!("[tray] run_on_main_thread(set_icon) 실패: {e}");
    }
}

// Toggle "부팅 시 자동 시작" (start automatically at boot) (ADR-0027 §55) — read whether it is currently enabled, invert it (enable/disable), and
// sync the CheckMenuItem check mark to the new state. Tray click · command (set_autostart) could share it,
// but the command takes an explicit bool, so the toggle (invert) is a tray-only entry.
//
// Failures of is_enabled/enable/disable·set_checked are only warn (one failed toggle must not kill the app). If the update is
// missed, the next click toggles again, so it is not stuck permanently.
pub fn toggle_autostart(app: &AppHandle) {
    let mgr = app.autolaunch();
    let enabled = match mgr.is_enabled() {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("[tray] autostart is_enabled 실패(토글 취소): {e}");
            return;
        }
    };
    let result = if enabled { mgr.disable() } else { mgr.enable() };
    if let Err(e) = result {
        tracing::warn!("[tray] autostart 토글(enable/disable) 실패: {e}");
        return;
    }
    let new_state = !enabled;
    if let Some(check) = app.try_state::<AutostartCheck>() {
        if let Err(e) = check.0.set_checked(new_state) {
            tracing::warn!("[tray] autostart CheckMenuItem set_checked 실패: {e}");
        }
    }
    tracing::info!(enabled = new_state, "[tray] 부팅 자동 시작 토글");
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use super::*;

    // R10: without a client quit skips the stop; with one it stops through the real client from a blocking thread (an
    // unconnected client finds no daemon to stop). The plan it picks is `quit_stops_on_the_exit_plan`'s.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn quit_stops_through_the_client_and_skips_without_one() {
        assert_eq!(stop_before_exit(None), None);
        let client = Arc::new(DaemonClient::unconnected(tokio::runtime::Handle::current()));
        let outcome = tokio::task::spawn_blocking(move || stop_before_exit(Some(&client)))
            .await
            .unwrap();
        assert_eq!(outcome, Some(StopOutcome::NoTarget));
    }

    #[test]
    fn quit_stops_on_the_exit_plan() {
        let client = super::super::tests::FakeClient::new(StopOutcome::Timeout);
        assert_eq!(
            stop_before_exit(Some(&client)),
            Some(StopOutcome::Timeout),
            "the outcome does not change the exit"
        );
        assert_eq!(client.calls(), ["stop:Exit:AppExit"]);
    }

    #[derive(Default)]
    struct Log(RefCell<Vec<String>>);

    impl UsageVisibility for Log {
        fn set_visible(&self, label: &str, visible: bool) {
            self.0.borrow_mut().push(format!("usage:{label}:{visible}"));
        }
    }

    const WINDOWS: [&str; 4] = ["main", "slot-popup-2", "devtools", "slot-popup-1"];

    // Show fixes the interest per window first and then calls the OS. A window that is not handled (`devtools`) is not told to interest either.
    #[test]
    fn showing_tells_the_usage_interest_before_each_os_call() {
        let log = Log::default();
        for_each_ui_window(
            WINDOWS,
            true,
            &log,
            |_| false,
            |_| {},
            |label| {
                log.0.borrow_mut().push(format!("os:{label}"));
                true
            },
        );
        assert_eq!(
            log.0.into_inner(),
            [
                "usage:slot-popup-1:true",
                "os:slot-popup-1",
                "usage:slot-popup-2:true",
                "os:slot-popup-2",
                "usage:main:true",
                "os:main",
            ]
        );
    }

    // Hide calls the OS per window first and removes only hidden windows from interest — a window whose hide failed stays as a visible window.
    #[test]
    fn hiding_marks_only_the_windows_the_os_actually_hid() {
        let log = Log::default();
        for_each_ui_window(
            WINDOWS,
            false,
            &log,
            |_| false,
            |_| {},
            |label| {
                log.0.borrow_mut().push(format!("os:{label}"));
                label != "slot-popup-2"
            },
        );
        assert_eq!(
            log.0.into_inner(),
            [
                "os:slot-popup-1",
                "usage:slot-popup-1:false",
                "os:slot-popup-2",
                "os:main",
                "usage:main:false",
            ]
        );
    }

    // A window of the spare-window pool is touched by neither the OS nor interest, in either show or hide.
    #[test]
    fn a_spare_window_is_left_alone_both_ways() {
        for visible in [true, false] {
            let log = Log::default();
            for_each_ui_window(
                WINDOWS,
                visible,
                &log,
                |label| label == "slot-popup-2",
                |_| {},
                |label| {
                    log.0.borrow_mut().push(format!("os:{label}"));
                    true
                },
            );
            assert!(
                log.0
                    .borrow()
                    .iter()
                    .all(|line| !line.contains("slot-popup-2")),
                "visible={visible}: {:?}",
                log.0.borrow()
            );
            assert!(log.0.borrow().contains(&"os:slot-popup-1".to_owned()));
        }
    }

    // ── Prewarmed windows — the hook · show · hide for skipped labels (TRD S21-slot-docking §8-1 ⑫) ──

    // The predicate is asked only once per popout label, and the hook receives the labels skipped by that decision once, before the first `os` call.
    #[test]
    fn the_skipped_hook_gets_the_spare_labels_once_before_any_os_call() {
        for visible in [true, false] {
            let log = Log::default();
            for_each_ui_window(
                [
                    "main",
                    "slot-popup-3",
                    "slot-popup-2",
                    "devtools",
                    "slot-popup-1",
                ],
                visible,
                &log,
                |label| {
                    log.0.borrow_mut().push(format!("spare:{label}"));
                    label != "slot-popup-1"
                },
                |skipped| {
                    log.0
                        .borrow_mut()
                        .push(format!("skipped:{}", skipped.join(",")))
                },
                |label| {
                    log.0.borrow_mut().push(format!("os:{label}"));
                    true
                },
            );
            let log = log.0.into_inner();
            assert_eq!(
                log[..4],
                [
                    "spare:slot-popup-3",
                    "spare:slot-popup-2",
                    "spare:slot-popup-1",
                    "skipped:slot-popup-3,slot-popup-2",
                ],
                "visible={visible}: {log:?}"
            );
            assert_eq!(
                log.iter()
                    .filter(|line| line.starts_with("spare:") || line.starts_with("skipped:"))
                    .count(),
                4,
                "visible={visible}: {log:?}"
            );
            assert!(
                log.contains(&"os:slot-popup-1".to_owned()) && log.contains(&"os:main".to_owned()),
                "visible={visible}: {log:?}"
            );
        }

        let calls = Cell::new(0);
        for_each_ui_window(
            ["main"],
            true,
            &Log::default(),
            |_| false,
            |skipped| {
                assert!(skipped.is_empty());
                calls.set(calls.get() + 1);
            },
            |_| true,
        );
        assert_eq!(calls.get(), 1, "건너뛴 것이 없어도 훅은 한 번 불린다");
    }

    // Show — among skipped prewarmed windows, only a window that left the pool in the meantime (`Adopted`) is shown with the same show as other windows, and since it comes before the windows whose order was decided,
    // main is last. A handed-over window (`Rehold`) · a prewarmed window not handed over (`Untouched`) are not shown.
    #[test]
    fn showing_reveals_only_the_spares_that_left_the_pool_before_the_ordered_windows() {
        let log = Log::default();
        show_ui_windows(
            [
                "main",
                "slot-popup-1",
                "slot-popup-2",
                "slot-popup-3",
                "slot-popup-4",
            ],
            &log,
            |label| label != "slot-popup-1",
            |label| {
                log.0.borrow_mut().push(format!("pool:{label}"));
                match label {
                    "slot-popup-2" => TrayShow::Rehold,
                    "slot-popup-3" => TrayShow::Adopted,
                    _ => TrayShow::Untouched,
                }
            },
            |label| {
                log.0.borrow_mut().push(format!("os:{label}"));
                true
            },
        );
        assert_eq!(
            log.0.into_inner(),
            [
                "pool:slot-popup-2",
                "pool:slot-popup-3",
                "usage:slot-popup-3:true",
                "os:slot-popup-3",
                "pool:slot-popup-4",
                "usage:slot-popup-1:true",
                "os:slot-popup-1",
                "usage:main:true",
                "os:main",
            ]
        );
    }

    // ── Show without stealing focus (TRD S26-window-chrome §6-2 · §10-1) ──

    /// A stand-in for the OS side of hidden windows — `held` is windows waiting for first paint · `ours` = our process holds the foreground. It
    /// blows up if an operation that belongs to the state command arrives (show-all does not call it).
    #[derive(Default)]
    struct Ops {
        held: Vec<&'static str>,
        ours: bool,
        calls: RefCell<Vec<String>>,
    }

    impl Ops {
        fn note(&self, call: &str, label: &str) {
            self.calls.borrow_mut().push(format!("{call}:{label}"));
        }
    }

    impl WindowOps for Ops {
        fn read(&self, label: &str) -> Option<crate::layout::window_ctl::OsWindowState> {
            Some(crate::layout::window_ctl::OsWindowState {
                awaiting_first_paint: self.held.iter().any(|held| *held == label),
                ..Default::default()
            })
        }
        fn minimize(&self, _: &str) -> Result<(), String> {
            unreachable!()
        }
        fn maximize(&self, _: &str) -> Result<(), String> {
            unreachable!()
        }
        fn unmaximize(&self, _: &str) -> Result<(), String> {
            unreachable!()
        }
        fn defer_maximized(&self, _: &str, _: bool) {
            unreachable!()
        }
        fn drop_deferred(&self, _: &str) {
            unreachable!()
        }
        fn raise_without_activation(&self, label: &str) {
            self.note("top", label);
        }
        fn show(&self, label: &str) -> Result<(), String> {
            self.note("show", label);
            Ok(())
        }
        fn unminimize(&self, label: &str) -> Result<(), String> {
            self.note("unminimize", label);
            Ok(())
        }
        fn apply_deferred(&self, label: &str) {
            self.note("apply", label);
        }
        fn foreground_is_ours(&self) -> bool {
            self.ours
        }
        fn set_focus(&self, label: &str) {
            self.note("focus", label);
        }
        fn show_all_no_steal(&self) {
            unreachable!()
        }
    }

    // Show in the same window order as the click branch (prewarmed windows that left the pool first · main last) but skip windows waiting for first paint, and give focus
    // only to main when the foreground is ours — if another app is in the foreground, it is given to no window.
    #[test]
    fn no_steal_skips_waiting_windows_and_focuses_only_main_when_the_foreground_is_ours() {
        for ours in [true, false] {
            let log = Log::default();
            let ops = Ops {
                held: vec!["slot-popup-1"],
                ours,
                ..Ops::default()
            };
            show_ui_windows_no_steal(
                ["main", "slot-popup-1", "slot-popup-2", "slot-popup-3"],
                &log,
                |label| label == "slot-popup-3",
                |_| TrayShow::Adopted,
                &ops,
            );
            let mut want: Vec<String> = ["slot-popup-3", "slot-popup-2", "main"]
                .iter()
                .flat_map(|label| {
                    ["top", "show", "apply"]
                        .iter()
                        .map(move |call| format!("{call}:{label}"))
                })
                .collect();
            if ours {
                want.push("focus:main".to_string());
            }
            assert_eq!(ops.calls.into_inner(), want, "ours={ours}");
            assert_eq!(
                log.0.into_inner(),
                [
                    "usage:slot-popup-3:true",
                    "usage:slot-popup-1:true",
                    "usage:slot-popup-2:true",
                    "usage:main:true",
                ],
                "ours={ours} — 대기 창도 곧 보인다"
            );
        }
    }

    // Hide — the raise of prewarmed windows skipped by the predicate is also withdrawn after the order. The withdrawal is all popout labels and does not ask the predicate again.
    #[test]
    fn hiding_cancels_the_hold_of_every_popup_label_after_the_ordered_windows() {
        let log = Log::default();
        hide_ui_windows(
            &WINDOWS,
            &log,
            |label| {
                log.0.borrow_mut().push(format!("spare:{label}"));
                label == "slot-popup-2"
            },
            |label| {
                log.0.borrow_mut().push(format!("os:{label}"));
                true
            },
            |label| log.0.borrow_mut().push(format!("cancel:{label}")),
        );
        assert_eq!(
            log.0.into_inner(),
            [
                "spare:slot-popup-2",
                "spare:slot-popup-1",
                "os:slot-popup-1",
                "usage:slot-popup-1:false",
                "os:main",
                "usage:main:false",
                "cancel:slot-popup-2",
                "cancel:slot-popup-1",
            ]
        );
    }
}
