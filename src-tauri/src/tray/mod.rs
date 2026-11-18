//! tray — the Tauri-integrated tray module (ADR-0026: tray integrated into the app, 2 processes). Windows only (tray GUI); `build_tray(app)` is called once in `setup()`.
// hangul-ok-begin
//! ## Structure (3 layers — CLAUDE.md 「코어 격리」 · 「LLM-우선 제어」)
// hangul-ok-end
//! - [`core`] — pure (no OS/Tauri/discovery): the MenuAction intent enum, menu_id↔action mapping, IconState, `to_grayscale_rgba`. The unit-test target.
//! - [`actions`] — impure shared side effects (show/hide windows, `quit_app`, `refresh_tray_icon`) so the tray handler and the command call the same function.
//! - This file (`mod.rs`) — Tauri wiring: TrayIconBuilder, menu creation, `on_menu_event` dispatch, creating · keeping the two icon sets (TrayIcons state) and the four menu item handles (TrayItems state — relabelled when the UI language changes).

pub mod actions;
pub mod core;

use std::sync::Arc;
use std::time::Duration;

use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Manager, Wry};

use engram_dashboard_protocol::DaemonInfo;
use tauri_plugin_autostart::ManagerExt;

use actions::{AutostartCheck, TrayDaemonClient, TRAY_ID};
use core::{IconState, MenuAction};

use crate::daemon_client::shutdown::{StopOrigin, StopOutcome, StopPlan};
use crate::daemon_client::DaemonClient;
use crate::ui_language::TrayMenuText;
use crate::ui_text::{tray_label, Lang};

// Two tray icon sets: color (daemon alive) · gray (dead). Created once in setup and managed as state — redoing the .ico decode/grayscale at every update is wasteful, so refresh's `set_icon` only swaps one of the two (`Image<'static>` is an internal Cow, clone is cheap; `set_icon` needs ownership).
pub struct TrayIcons {
    pub active: Image<'static>,
    pub inactive: Image<'static>,
}

const ICON_ICO: &[u8] = include_bytes!("../../icons/icon.ico");

const ENSURE_TIMEOUT: Duration = Duration::from_secs(5);

// Create the two tray icon sets. Color = .ico decode via Tauri `Image::from_bytes` (image-ico feature, no separate image crate); gray = the decoded RGBA converted with `core::to_grayscale_rgba` and rebuilt with `Image::new_owned`.
fn build_icons() -> tauri::Result<TrayIcons> {
    let active = Image::from_bytes(ICON_ICO)?;
    let (w, h) = (active.width(), active.height());
    let gray_rgba = core::to_grayscale_rgba(active.rgba(), w, h);
    let inactive = Image::new_owned(gray_rgba, w, h);
    let active = active.to_owned();
    Ok(TrayIcons { active, inactive })
}

/// The four menu item handles — kept so a `ui.language` change can relabel them ([`TauriTrayText`]). [`build_tray`] manages them
/// right after it makes the menu, before `TrayIconBuilder::build` — so they stay managed even if that last call fails.
// ADR-0373
pub(crate) struct TrayItems {
    start: MenuItem<Wry>,
    stop: MenuItem<Wry>,
    autostart: CheckMenuItem<Wry>,
    quit: MenuItem<Wry>,
}

impl TrayItems {
    // Called on the main thread, where `set_text` runs in place (from another thread it would wait for the main thread).
    fn relabel(&self, lang: Lang) {
        let results = [
            (
                MenuAction::StartDaemon,
                self.start
                    .set_text(tray_label(lang, MenuAction::StartDaemon)),
            ),
            (
                MenuAction::StopDaemon,
                self.stop.set_text(tray_label(lang, MenuAction::StopDaemon)),
            ),
            (
                MenuAction::ToggleAutostart,
                self.autostart
                    .set_text(tray_label(lang, MenuAction::ToggleAutostart)),
            ),
            (
                MenuAction::QuitApp,
                self.quit.set_text(tray_label(lang, MenuAction::QuitApp)),
            ),
        ];
        for (action, result) in results {
            if let Err(e) = result {
                tracing::warn!(
                    module = "tray",
                    item = action.menu_id(),
                    "could not relabel a menu item: {e}"
                );
            }
        }
    }
}

/// The tray side of a `ui.language` change — posts the relabel to the main thread and returns without waiting, so the settings reply is not held by a busy main thread. Main-thread posts run first in, first out, so they land in the hook's order.
pub(crate) struct TauriTrayText<'a>(pub &'a AppHandle);

impl TrayMenuText for TauriTrayText<'_> {
    fn relabel(&self, lang: Lang) {
        let app = self.0.clone();
        let posted = self.0.run_on_main_thread(move || {
            // Looked up when the post runs (after setup), so a change racing `build_tray` still finds the items. No items = `build_tray` failed before it made the menu (tolerated — the app runs without a tray).
            if let Some(items) = app.try_state::<TrayItems>() {
                items.relabel(lang);
            }
        });
        if let Err(e) = posted {
            tracing::warn!(
                module = "tray",
                lang = lang.code(),
                "could not post the menu relabel to the main thread: {e}"
            );
        }
    }
}

// Create and wire the tray (once in setup): manage the two icon sets and attach the menu · handler. Menu order: daemon on / daemon off / start at boot / ──separator── / full quit. Ids come from `core::MenuAction` (pure), labels from `ui_text::tray_label` in the effective UI language; click → `action_for_menu_id` → dispatch. No menu item for UI show·hide — show is the tray left click (single, `on_tray_icon_event`), hide is main's X.
pub fn build_tray(app: &App) -> tauri::Result<()> {
    let icons = build_icons()?;
    let initial = icons.inactive.clone();
    app.manage(icons);

    // ── Menu items (core's id · ui_text's label — pure separation) ──────────────────────────────
    let handle = app.handle();
    let lang = crate::ui_language::current(handle);
    let mi = |a: MenuAction| {
        MenuItem::with_id(handle, a.menu_id(), tray_label(lang, a), true, None::<&str>)
    };
    let start = mi(MenuAction::StartDaemon)?;
    let stop = mi(MenuAction::StopDaemon)?;
    // ADR-0027 §55: autostart is a checkable item (CheckMenuItem), initial check = whether it is registered in the registry. The registration argument (`--hidden`, no mode argument — ADR-0029) is embedded in the autostart plugin init; here only the enabled state is read into the check.
    let autostart_action = MenuAction::ToggleAutostart;
    let autostart_checked = app.autolaunch().is_enabled().unwrap_or(false);
    let autostart = CheckMenuItem::with_id(
        handle,
        autostart_action.menu_id(),
        tray_label(lang, autostart_action),
        true,
        autostart_checked,
        None::<&str>,
    )?;
    let sep = PredefinedMenuItem::separator(handle)?;
    let quit = mi(MenuAction::QuitApp)?;
    // Order: daemon on/off/start at boot/(separator)/full quit — matches core::ALL.
    let menu = Menu::with_items(handle, &[&start, &stop, &autostart, &sep, &quit])?;

    app.manage(AutostartCheck(autostart.clone()));
    app.manage(TrayItems {
        start,
        stop,
        autostart,
        quit,
    });

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(initial)
        .tooltip("Engram")
        .menu(&menu)
        // Menu = right click; left click (single) opens the UI (`on_tray_icon_event` below), so the menu must not appear on it (user request). The default is true, so false is explicit (Tauri 2.11: `menu_on_left_click` is deprecated → `show_menu_on_left_click`).
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| dispatch_menu(app, event.id.as_ref()))
        // Tray left click (single) = bring back all UI windows (user request, ADR-0229). Only Left + Up (release) is handled — a click fires Down and Up, so Up alone avoids showing twice; right click/Enter/Move/Leave are ignored. `show_main_ui` shares the actions function with the command · single-instance relaunch (CLAUDE.md 「LLM-우선 제어」). A person's // hangul-ok
        // press, so it is the 「click」 branch (TRD S26-window-chrome §6-2 「which branch is whose」).
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                actions::show_main_ui(
                    tray.app_handle(),
                    crate::layout::window_ctl::Activation::TrayClick,
                );
            }
        })
        .build(app)?;

    actions::refresh_tray_icon(&app.handle().clone());
    Ok(())
}

// The periodic daemon-liveness observer (spawned once in setup). Fixes the stuck-gray bug: even if the daemon starts or dies externally after boot, the tray icon · emit follow within ≤PROBE_INTERVAL.
// Periodic probe = ADR-0028 "observer impl behind the bus (pre-flip)": this polling is not a temporary workaround but the single push source Rust owns. At the flip stage, when liveness comes directly from daemon connection (WS) events, the probe mechanism is replaced by those events but the bus/subscriber shape — "the Rust observer detects a change and pushes to the tray (native) and the WebView (emit)" — is unchanged; the frontend later subscribes to `daemon-status-changed` (ADR-0028 single push channel).
// Division of labor with the StopOutcome immediate-confirm branch: the stuck-gray race right after turn-off is blocked by dispatch's `icon_state_for_stop_outcome` (immediate gray bypassing the probe); this observer complements it — immediacy is StopOutcome's, external changes (daemon appearing after boot · crash) are the observer's within ≤PROBE_INTERVAL.
// Error isolation: neither probe nor publish may kill the observer thread, so results are ignored (errors are internal warns) and it loops forever (a panic is not caught — the hook turns a shell panic into a shell exit, ADR-0292). Change decision · duplicate blocking · suppression window belong to `publish_daemon_liveness` (LivenessState); the observer only does probe→publish. The initial icon is set by build_tray's refresh with publish first setting `last`, so the observer's first loop giving the same value is ignored (no change).
pub fn spawn_daemon_observer(app: &AppHandle) {
    const PROBE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3);

    let app = app.clone();
    std::thread::Builder::new()
        .name("engram-daemon-observer".into())
        .spawn(move || {
            let data_dir = crate::discovery::default_data_dir();
            loop {
                let alive = crate::discovery::daemon_status(&data_dir).alive;
                // ADR-0028
                actions::publish_daemon_liveness(&app, alive);
                std::thread::sleep(PROBE_INTERVAL);
            }
        })
        .map_err(|e| tracing::warn!("[tray] daemon observer thread spawn failed: {e}"))
        .ok();
}

// Menu click id → MenuAction → action; every action calls only `actions` (shared side effects).
// Daemon on/off blocks for seconds (ensure_daemon WMI polling · the stop flight's bound) and `on_menu_event` is on the main thread, so a direct call would freeze the tray · windows — it goes to a `spawn_blocking` worker and on completion updates the icon with `refresh_tray_icon` (main-thread `set_icon`).
fn dispatch_menu(app: &AppHandle, menu_id: &str) {
    let Some(action) = core::action_for_menu_id(menu_id) else {
        tracing::warn!("[tray] unknown menu id: {menu_id}");
        return;
    };
    match action {
        MenuAction::StartDaemon => spawn_daemon_action(app, DaemonOp::Start),
        MenuAction::StopDaemon => spawn_daemon_action(app, DaemonOp::Stop),
        MenuAction::QuitApp => actions::quit_app(app),
        MenuAction::ToggleAutostart => actions::toggle_autostart(app),
    }
}

enum DaemonOp {
    Start,
    Stop,
}

fn spawn_daemon_action(app: &AppHandle, op: DaemonOp) {
    let app = app.clone();
    // A worker panic is not guarded (intentional): in the production shell the hook turns a panic in the closure below into a shell exit (ADR-0292), so an RAII drop guard (SignalOnDrop) to revive the icon update has no place. In a test build without the hook a panic before set/refresh only misses that click's icon update, and the next on/off click's probe recovers the state.
    tauri::async_runtime::spawn_blocking(move || {
        let client = app
            .try_state::<Arc<DaemonClient>>()
            .map(|client| client.inner().clone());
        let client = client
            .as_ref()
            .map(|client| client as &dyn TrayDaemonClient);
        match op {
            DaemonOp::Start => {
                start_daemon(client, launch_daemon);
                actions::refresh_tray_icon(&app);
            }
            DaemonOp::Stop => match stop_daemon(client) {
                AfterStop::ForceDown => actions::force_daemon_down(&app),
                AfterStop::Probe => actions::refresh_tray_icon(&app),
            },
        }
    });
}

/// Tray 「Start daemon」 (R10): `launch`, then the attach even if the launch failed or timed out — a daemon that comes up later is found by the link's background retries. A stop in flight is not waited out (`daemon_client::shutdown`).
fn start_daemon(
    client: Option<&dyn TrayDaemonClient>,
    launch: impl FnOnce() -> Result<DaemonInfo, String>,
) {
    match launch() {
        Ok(info) => tracing::info!(pid = info.pid, port = info.port, "[tray] daemon ensure done"),
        Err(e) => tracing::warn!("[tray] {e}"),
    }
    match client {
        Some(client) => client.attach_in_background(),
        None => tracing::warn!("[tray] no daemon client — not attaching after the start"),
    }
}

/// Finds the daemon exe and starts it if none runs — blocks up to [`ENSURE_TIMEOUT`]. `Err` = the log text. Never spawn the daemon directly with `std::process::Command` (ADR-0024 C1): `ensure_daemon` spawns only via WMI (`Win32_Process.Create`), as a child of WmiPrvSE that does not inherit the app Job (KILL_ON_JOB_CLOSE) — the detached/breakaway requirement is met automatically; a direct spawn would kill the daemon with the app on exit.
fn launch_daemon() -> Result<DaemonInfo, String> {
    let exe =
        crate::discovery::locate_daemon_exe().map_err(|e| format!("daemon exe lookup failed: {e}"))?;
    // console=false: windowless. Never log the token (discovery guarantees it).
    crate::discovery::ensure_daemon(
        &crate::discovery::default_data_dir(),
        &exe,
        ENSURE_TIMEOUT,
        false,
    )
    .map_err(|e| format!("daemon ensure failed: {e}"))
}

#[derive(Debug, PartialEq, Eq)]
enum AfterStop {
    ForceDown,
    Probe,
}

/// Tray 「Stop daemon」 (R10) — the `User` plan; with no client the stop is skipped with a warn and the icon is probed. The icon after a stop branches on [`StopOutcome`] (prevents the S13/M-1 race): `Closed` / `Killed` = the daemon process is gone → [`AfterStop::ForceDown`] (bypasses the probe race); `Timeout` / `NoTarget` = uncertain → the probe.
fn stop_daemon(client: Option<&dyn TrayDaemonClient>) -> AfterStop {
    let Some(client) = client else {
        tracing::warn!("[tray] no daemon client — the daemon stop is skipped");
        return AfterStop::Probe;
    };
    let outcome = client.stop(StopPlan::User, StopOrigin::Tray);
    tracing::info!(?outcome, "[tray] daemon stop done");
    if is_daemon_down(outcome) {
        AfterStop::ForceDown
    } else {
        AfterStop::Probe
    }
}

// [`StopOutcome`] → the icon state after turn-off (impure layer — depends on the `daemon_client::shutdown` type). Kept here, not in core.rs (zero tauri/discovery imports, knows only IconState), which would lose purity.
// - `Closed | Killed` (the stop saw the daemon process gone — R12 B2: a kill maps like a close) → `Some(Inactive)`: gray confirmed (on the turn-off path `force_daemon_down` also sets the death-window suppression — prevents the M-1 race).
// - `Timeout | NoTarget` (uncertain/no daemon to turn off) → `None`: the caller takes the existing probe fallback (refresh).
fn icon_state_for_stop_outcome(outcome: StopOutcome) -> Option<IconState> {
    match outcome {
        StopOutcome::Closed | StopOutcome::Killed => Some(IconState::Inactive),
        StopOutcome::Timeout | StopOutcome::NoTarget => None,
    }
}

// Whether the turn-off result is "the daemon is gone" — reuses `icon_state_for_stop_outcome`; gone → `force_daemon_down` (gray + suppression window), else the probe fallback.
fn is_daemon_down(outcome: StopOutcome) -> bool {
    icon_state_for_stop_outcome(outcome).is_some()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use engram_dashboard_protocol::PROTOCOL_VERSION;

    use super::*;

    pub(super) struct FakeClient {
        calls: Arc<Mutex<Vec<String>>>,
        outcome: StopOutcome,
    }

    impl FakeClient {
        pub(super) fn new(outcome: StopOutcome) -> Self {
            Self {
                calls: Arc::default(),
                outcome,
            }
        }

        pub(super) fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        fn note(&self, call: String) {
            self.calls.lock().unwrap().push(call);
        }
    }

    impl TrayDaemonClient for FakeClient {
        fn stop(&self, plan: StopPlan, origin: StopOrigin) -> StopOutcome {
            self.note(format!("stop:{plan:?}:{origin:?}"));
            self.outcome
        }

        fn attach_in_background(&self) {
            self.note("attach".into());
        }
    }

    #[test]
    fn start_launches_then_attaches_even_after_a_failed_launch() {
        let launched: [Result<DaemonInfo, String>; 2] = [
            Ok(crate::discovery::tests::info(1001, PROTOCOL_VERSION)),
            Err("daemon ensure failed: timed out".into()),
        ];
        for launch in launched {
            let client = FakeClient::new(StopOutcome::NoTarget);
            let calls = client.calls.clone();
            start_daemon(Some(&client), move || {
                calls.lock().unwrap().push("launch".into());
                launch
            });
            assert_eq!(client.calls(), ["launch", "attach"]);
        }
    }

    #[test]
    fn start_without_a_client_still_launches() {
        let mut launched = false;
        start_daemon(None, || {
            launched = true;
            Err("daemon exe lookup failed: none".into())
        });
        assert!(launched);
    }

    #[test]
    fn stop_runs_the_user_plan_and_forces_gray_only_when_the_daemon_is_gone() {
        for (outcome, after) in [
            (StopOutcome::Closed, AfterStop::ForceDown),
            (StopOutcome::Killed, AfterStop::ForceDown),
            (StopOutcome::Timeout, AfterStop::Probe),
            (StopOutcome::NoTarget, AfterStop::Probe),
        ] {
            let client = FakeClient::new(outcome);
            assert_eq!(stop_daemon(Some(&client)), after, "{outcome:?}");
            assert_eq!(client.calls(), ["stop:User:Tray"]);
        }
    }

    #[test]
    fn stop_without_a_client_skips_and_probes() {
        assert_eq!(stop_daemon(None), AfterStop::Probe);
    }

    #[test]
    fn a_closed_or_killed_stop_forces_gray_others_fall_back() {
        // Closed / Killed = the daemon process is gone → set gray (Inactive) directly (bypassing the probe).
        assert_eq!(
            icon_state_for_stop_outcome(StopOutcome::Closed),
            Some(IconState::Inactive),
        );
        assert_eq!(
            icon_state_for_stop_outcome(StopOutcome::Killed),
            Some(IconState::Inactive),
        );
        // Timeout/NoTarget = uncertain → None (the caller does a probe-fallback refresh).
        assert_eq!(icon_state_for_stop_outcome(StopOutcome::Timeout), None);
        assert_eq!(icon_state_for_stop_outcome(StopOutcome::NoTarget), None);
    }
}
