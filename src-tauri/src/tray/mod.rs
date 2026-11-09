//! tray — the Tauri-integrated tray module (ADR-0026: tray integrated into the app, 2 processes).
//!
//! ## Structure (3 layers — CLAUDE.md 「코어 격리」 · 「LLM-우선 제어」)
//! - [`core`] — **pure** (no OS/Tauri/discovery dependence): the MenuAction intent enum, menu_id↔action mapping,
//!   IconState, to_grayscale_rgba. The target of unit tests.
//! - [`actions`] — **impure shared side effects**: so the tray handler and the command call the same function (no duplication).
//!   show/hide windows, quit_app, refresh_tray_icon.
//! - This file (`mod.rs`) — Tauri wiring (impure): TrayIconBuilder, menu creation, on_menu_event dispatch,
//!   creating·keeping the two icon sets (TrayIcons state).
//!
//! Windows only (tray GUI). build_tray(app) is called once in setup().

pub mod actions;
pub mod core;

use std::sync::Arc;
use std::time::Duration;

use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Manager};

use engram_dashboard_protocol::DaemonInfo;
use tauri_plugin_autostart::ManagerExt;

use actions::{AutostartCheck, TrayDaemonClient, TRAY_ID};
use core::{IconState, MenuAction};

use crate::daemon_client::shutdown::{StopOrigin, StopOutcome, StopPlan};
use crate::daemon_client::DaemonClient;

// Two tray icon sets: color (daemon alive) · gray (dead). Created once in setup and managed as state.
//
// ★Created once and kept (load-bearing)★: redoing the .ico decode/grayscale conversion at every update is wasteful, so
// setup makes the two sets and holds them, and on refresh set_icon only swaps one of the two. Image<'static>
// is an internal Cow so clone is cheap (set_icon requires ownership, so it is cloned on update).
pub struct TrayIcons {
    pub active: Image<'static>,
    pub inactive: Image<'static>,
}

// The built-in color icon (.ico). Embedded at compile time to remove path dependence at deployment (the same way as tray-host).
const ICON_ICO: &[u8] = include_bytes!("../../icons/icon.ico");

// The blocking limit of ensure_daemon (WMI spawn + polling). 5 seconds, the same as discovery's internal polling timeout.
const ENSURE_TIMEOUT: Duration = Duration::from_secs(5);

// Create the two tray icon sets. Color = .ico decode, gray = grayscale conversion of the color RGBA.
//
// The .ico → RGBA decode is done with Tauri Image::from_bytes (image-ico feature) (no separate image crate needed).
// For gray, the decoded RGBA buffer is converted with core::to_grayscale_rgba and rebuilt with Image::new_owned.
fn build_icons() -> tauri::Result<TrayIcons> {
    // The image-ico feature is required (without it, the .ico decode fails).
    let active = Image::from_bytes(ICON_ICO)?;
    let (w, h) = (active.width(), active.height());
    // to_grayscale_rgba assumes len==w*h*4 (guaranteed by the decode).
    let gray_rgba = core::to_grayscale_rgba(active.rgba(), w, h);
    let inactive = Image::new_owned(gray_rgba, w, h);
    // Color is also promoted to owned (Image::from_bytes is 'static — OK since ICON_ICO is 'static, explicit clone).
    let active = active.to_owned();
    Ok(TrayIcons { active, inactive })
}

// Create and wire the tray (once in setup). Manage the two icon sets and attach the menu · handler.
//
// Menu (order): daemon on / daemon off / start at boot / ──separator── / full quit.
// Menu ids and labels come from core::MenuAction (pure). Click → action_for_menu_id → dispatch.
// There are no menu items for UI show·hide — show is **tray left click (single)** (on_tray_icon_event), and hide is main's X.
pub fn build_tray(app: &App) -> tauri::Result<()> {
    let icons = build_icons()?;
    // Initial icon = gray (the daemon state is confirmed by the refresh right after setup).
    let initial = icons.inactive.clone();
    app.manage(icons);

    // ── Menu items (using core's id/label — pure separation) ──────────────────────────────────
    let handle = app.handle();
    let mi = |a: MenuAction| MenuItem::with_id(handle, a.menu_id(), a.label(), true, None::<&str>);
    let start = mi(MenuAction::StartDaemon)?;
    let stop = mi(MenuAction::StopDaemon)?;
    // ADR-0027 §55: autostart is a checkable item (CheckMenuItem). Initial check = whether it is currently registered in the registry.
    // The registration argument (--hidden, no mode argument — ADR-0029) was embedded in the autostart plugin init,
    // and here only the enabled state is read and reflected in the check.
    let autostart_action = MenuAction::ToggleAutostart;
    let autostart_checked = app.autolaunch().is_enabled().unwrap_or(false);
    let autostart = CheckMenuItem::with_id(
        handle,
        autostart_action.menu_id(),
        autostart_action.label(),
        true,
        autostart_checked,
        None::<&str>,
    )?;
    let sep = PredefinedMenuItem::separator(handle)?;
    let quit = mi(MenuAction::QuitApp)?;
    // Order: daemon on/off/start at boot/(separator)/full quit — matches core::ALL.
    let menu = Menu::with_items(handle, &[&start, &stop, &autostart, &sep, &quit])?;

    // Keep the CheckMenuItem handle for set_checked sync on toggle (actions::toggle_autostart looks it up again).
    app.manage(AutostartCheck(autostart));

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(initial)
        .tooltip("Engram")
        .menu(&menu)
        // So the menu does not appear on left click (user request). Menu = right click, left click (single) = open UI (on_tray_icon_event below).
        // The default is true so false is explicit. (Tauri 2.11: menu_on_left_click is deprecated → use show_.)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| dispatch_menu(app, event.id.as_ref()))
        // Tray left click (single) = bring back all UI windows (user request, ADR-0229). Only Left + Up (on release) is handled —
        // Click fires both Down and Up, so it is limited to Up to fire only once (prevents duplicate showing). Right click/Enter/Move/
        // Leave are ignored. show_main_ui shares the same actions function as the command · single-instance relaunch (CLAUDE.md 「LLM-우선 제어」). A person's
        // press, so it is the 「누름」 (click) branch (TRD S26-window-chrome §6-2 「누가 어느 갈래인가」).
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

// The periodic daemon-liveness observer (spawned once in setup). Resolves the stuck-gray bug — even if the daemon is
// started or dies externally after boot, the tray icon · emit follow within ≤PROBE_INTERVAL.
//
// ★Periodic probe = ADR-0028 "observer impl behind the bus (pre-flip)" (load-bearing)★: this polling is not a temporary workaround
// but **the single push source owned by Rust**. At the flip stage, when liveness is received directly from daemon connection (WS) events, the
// *probe mechanism* is replaced by those events, but the **bus/subscriber shape** — "the Rust observer detects a change and pushes to two subscribers,
// the tray (native) and the WebView (emit)" — **is unchanged**. The frontend later subscribes to
// `daemon-status-changed` (ADR-0028 single push channel).
//
// ★Division of labor with the StopOutcome immediate-confirm branch★: the stuck-gray race right after turn-off is blocked by dispatch's
// icon_state_for_stop_outcome (immediate gray bypassing the probe), and this observer **complements it rather than replacing it**
// — immediacy is StopOutcome's, and external changes (daemon appearing after boot · crash) are reflected by the observer within ≤PROBE_INTERVAL.
//
// Error isolation: neither probe nor publish may kill the observer thread, so results are ignored (errors are internal warns)
// and it loops forever (a panic is not caught — a panic in the shell is turned into a shell exit right away by the hook · ADR-0292).
// Change decision · duplicate blocking · suppression window are owned by publish_daemon_liveness (LivenessState) — the observer only does probe→publish.
// The initial icon is set by build_tray refresh with publish first setting last, so if the observer's first loop gives the same value,
// publish automatically ignores it (no change).
pub fn spawn_daemon_observer(app: &AppHandle) {
    // Probe period. A compromise between responsiveness (delay in reflecting external changes) and load (daemon.json read + PID probe) — 3 seconds.
    const PROBE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3);

    let app = app.clone();
    // A detached background thread (resident for the whole app lifetime). Independent of the tokio runtime (std::thread + sleep).
    std::thread::Builder::new()
        .name("engram-daemon-observer".into())
        .spawn(move || {
            // m-4: data_dir is an immutable value (default_data_dir), so compute it only once outside the loop and cache it.
            let data_dir = crate::discovery::default_data_dir();
            loop {
                let alive = crate::discovery::daemon_status(&data_dir).alive;
                // Feed it into the single publish entry point — change decision · suppression window (M-1) · tray set+emit unified (ADR-0028).
                actions::publish_daemon_liveness(&app, alive);
                std::thread::sleep(PROBE_INTERVAL);
            }
        })
        .map_err(|e| tracing::warn!("[tray] 데몬 옵저버 스레드 spawn 실패: {e}"))
        .ok();
}

// Menu click id → MenuAction → action. Every action calls only actions (shared side effects).
//
// ★Daemon on/off = blocking → worker (load-bearing)★: ensure_daemon and the stop block for seconds (WMI polling · the stop
// flight's bound).
// on_menu_event is on the main thread, so calling it directly freezes the tray · windows meanwhile → send it to a spawn_blocking
// worker and, on completion, update the icon with refresh_tray_icon (main-thread set_icon).
fn dispatch_menu(app: &AppHandle, menu_id: &str) {
    let Some(action) = core::action_for_menu_id(menu_id) else {
        tracing::warn!("[tray] 알 수 없는 메뉴 id: {menu_id}");
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

// Run daemon on/off in a worker and update the icon on completion.
fn spawn_daemon_action(app: &AppHandle, op: DaemonOp) {
    let app = app.clone();
    // ★A worker panic is not guarded (intentional)★: in the production shell a panic in the closure below is turned into a shell exit by the hook
    // (ADR-0292) — there is no place for an RAII drop guard (SignalOnDrop) to revive the icon update. In a test build without the hook,
    // a panic before reaching set/refresh only misses that click's icon update, and the probe of the next on/off click
    // recovers the state.
    tauri::async_runtime::spawn_blocking(move || {
        // R10: the stop path runs on the daemon client's runtime — none when setup failed to build the client.
        let client = app
            .try_state::<Arc<DaemonClient>>()
            .map(|client| client.inner().clone());
        let client = client
            .as_ref()
            .map(|client| client as &dyn TrayDaemonClient);
        match op {
            DaemonOp::Start => {
                start_daemon(client, launch_daemon);
                // A probe suits confirming alive (posted to the main thread set_icon).
                actions::refresh_tray_icon(&app);
            }
            DaemonOp::Stop => match stop_daemon(client) {
                AfterStop::ForceDown => actions::force_daemon_down(&app),
                AfterStop::Probe => actions::refresh_tray_icon(&app),
            },
        }
    });
}

/// Tray 「데몬 켜기」 (R10): a stop in flight ends first — otherwise this start could attach to the daemon it is
/// stopping — then `launch`, then the attach even when the launch failed or timed out: a daemon that comes up later is
/// found by the link's background retries.
fn start_daemon(
    client: Option<&dyn TrayDaemonClient>,
    launch: impl FnOnce() -> Result<DaemonInfo, String>,
) {
    if let Some(client) = client {
        client.wait_stop_over();
    }
    match launch() {
        Ok(info) => tracing::info!(pid = info.pid, port = info.port, "[tray] 데몬 ensure 완료"),
        Err(e) => tracing::warn!("[tray] {e}"),
    }
    match client {
        Some(client) => client.attach_in_background(),
        None => tracing::warn!("[tray] no daemon client — not attaching after the start"),
    }
}

/// Finds the daemon exe and starts it if none runs — blocks up to [`ENSURE_TIMEOUT`]. `Err` = the log text.
///
/// ★Do not spawn the daemon directly with std::process::Command (ADR-0024 C1)★: ensure_daemon spawns only via WMI
/// (Win32_Process.Create) inside — as a child of WmiPrvSE it does not inherit the app Job (KILL_ON_JOB_CLOSE) = the
/// detached/breakaway requirement is met automatically. A direct Command spawn here would kill the daemon along with the
/// app on exit.
fn launch_daemon() -> Result<DaemonInfo, String> {
    let exe =
        crate::discovery::locate_daemon_exe().map_err(|e| format!("daemon exe 탐색 실패: {e}"))?;
    // console=false: windowless. Never log the token (discovery guarantees it).
    crate::discovery::ensure_daemon(
        &crate::discovery::default_data_dir(),
        &exe,
        ENSURE_TIMEOUT,
        false,
    )
    .map_err(|e| format!("데몬 ensure 실패: {e}"))
}

/// The icon update after 「데몬 끄기」.
#[derive(Debug, PartialEq, Eq)]
enum AfterStop {
    /// The daemon process is gone — `force_daemon_down` (gray and the suppression window).
    ForceDown,
    /// Uncertain, or nothing was stopped — the probe (`refresh_tray_icon`).
    Probe,
}

/// Tray 「데몬 끄기」 (R10) — the `User` plan; with no client the stop is skipped with a warn and the icon is probed.
///
/// ★The icon after a stop branches on [`StopOutcome`] (load-bearing — prevents the S13/M-1 race)★: `Closed` / `Killed`
/// = the daemon process is gone → [`AfterStop::ForceDown`], which bypasses the probe race; `Timeout` / `NoTarget` =
/// uncertain → the probe.
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

// [`StopOutcome`] → the icon state after turn-off (impure layer — depends on the `daemon_client::shutdown` type).
//
// ★Why it is here and not in core (load-bearing)★: StopOutcome is the type of `daemon_client::shutdown`, which turns the
// daemon off, so putting it in core.rs (zero tauri/discovery imports, knows only IconState) would break purity. So only
// this mapping is placed in the impure layer.
// - `Closed | Killed` (the stop saw the daemon process gone — R12 B2: a kill maps like a close) → `Some(Inactive)`: gray
//   confirmed (on the turn-off path, force_daemon_down sets even the death-window suppression window on top of this
//   confirmation — prevents the M-1 race).
// - `Timeout | NoTarget` (uncertain/no daemon to turn off) → `None`: the caller takes the existing probe fallback (refresh).
fn icon_state_for_stop_outcome(outcome: StopOutcome) -> Option<IconState> {
    match outcome {
        StopOutcome::Closed | StopOutcome::Killed => Some(IconState::Inactive),
        StopOutcome::Timeout | StopOutcome::NoTarget => None,
    }
}

// Whether the turn-off result is "the daemon is gone". Reuses the mapping (icon_state_for_stop_outcome) and is used for
// the turn-off path branch — if gone, force_daemon_down (gray + suppression window), otherwise the probe fallback.
fn is_daemon_down(outcome: StopOutcome) -> bool {
    icon_state_for_stop_outcome(outcome).is_some()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use engram_dashboard_protocol::PROTOCOL_VERSION;

    use super::*;

    /// A daemon client that records what the tray asked of it, in order, with the calls `launch` adds.
    pub(super) struct FakeClient {
        calls: Arc<Mutex<Vec<String>>>,
        outcome: StopOutcome,
    }

    impl FakeClient {
        /// Its stops end with `outcome`.
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
        fn wait_stop_over(&self) {
            self.note("wait_stop_over".into());
        }

        fn stop(&self, plan: StopPlan, origin: StopOrigin) -> StopOutcome {
            self.note(format!("stop:{plan:?}:{origin:?}"));
            self.outcome
        }

        fn attach_in_background(&self) {
            self.note("attach".into());
        }
    }

    #[test]
    fn start_waits_for_the_stop_then_launches_then_attaches_even_after_a_failed_launch() {
        let launched: [Result<DaemonInfo, String>; 2] = [
            Ok(crate::discovery::tests::info(1001, PROTOCOL_VERSION)),
            Err("데몬 ensure 실패: timed out".into()),
        ];
        for launch in launched {
            let client = FakeClient::new(StopOutcome::NoTarget);
            let calls = client.calls.clone();
            start_daemon(Some(&client), move || {
                calls.lock().unwrap().push("launch".into());
                launch
            });
            assert_eq!(client.calls(), ["wait_stop_over", "launch", "attach"]);
        }
    }

    #[test]
    fn start_without_a_client_still_launches() {
        let mut launched = false;
        start_daemon(None, || {
            launched = true;
            Err("daemon exe 탐색 실패: none".into())
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
