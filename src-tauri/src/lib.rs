// Window chrome — the OS window title (TRD S26-window-chrome §4-4).
mod chrome;
pub mod commands;
pub mod daemon_client;
pub mod discovery;
mod file_hooks;
mod first_paint;
pub mod layout;
pub mod output_channel;
pub mod output_router;
pub mod settings;
pub mod spare;
pub mod state;
// The shell's part of tab drop — the one drop command after a drag ends · window facts (ADR-0296).
pub mod tab_drop;
pub mod theme;
mod tray;
// The shell-made UI text in both UI languages, and applying a `ui.language` change to it (ADR-0373).
mod ui_language;
pub mod ui_text;
// ADR-0155: the shell-side leg of commands owned by the webview (registration proxy + the last hop of the two-stage delivery).
pub mod view_commands;
mod webview_env;

// ADR-0029: the app is the daemon's resident client shell (window/tray/local control commands + daemon discovery); the daemon hosts the agents.
use tauri::Manager;

/// The process exit code of a start that could not create the main window (setup ⑧).
const STARTUP_FAILED_EXIT_CODE: i32 = 1;

// Daemon agent list → the tab auto-name ledger; the connection task calls it after the frontend notice. The layout must be the same instance as human clicks and the bus (ADR-0035: one layout authority). (TRD S26-window-chrome §4-3)
struct LayoutAgentNames {
    layout: crate::layout::LayoutState,
    app: tauri::AppHandle,
}

impl crate::daemon_client::AgentNamesSink for LayoutAgentNames {
    fn agents(&self, agents: &[engram_dashboard_protocol::AgentInfo]) {
        let names: Vec<(String, String)> = agents
            .iter()
            .map(|agent| (agent.id.to_string(), agent.name.clone()))
            .collect();
        if let Err(e) = crate::layout::apply::agent_names_updated(
            &self.layout,
            &crate::commands::layout::TauriEvents { app: &self.app },
            &names,
        ) {
            tracing::warn!("could not update the agent name ledger (tab names follow from the next list): {e}");
        }
    }
}

// ── run() ────────────────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // ★Before anything else★: every panic after this leaves a reason line and ends the shell. The file log is opened later by the boot plugin (`state/boot_plugin.rs` ⓪), so an earlier panic leaves no line in the file (only the std default hook's stderr; release builds have no console).
    // ADR-0292
    engram_dashboard_base::panic::install_hook();

    // ADR-0029: a boot launch (autostart registration arguments include --hidden) keeps only the tray resident, without a window.
    let hidden = std::env::args().any(|a| a == "--hidden");

    // ── Screen state (TRD S21-storage §6) — the boot-stage plugin · user setup · exit see the same instance ──
    let layout = crate::layout::LayoutState::new();
    let labels = std::sync::Arc::new(crate::commands::popout::PopupCounter::default());
    let state_session = std::sync::Arc::new(crate::state::boot_plugin::StateSession::default());
    let restore = std::sync::Arc::new(crate::state::restore::RestoreService::new());
    // The single path that answers the crash copy (TRD S21-storage §6-7) — humans (the Tauri wrapper) and LLMs (the bus) see the same instance.
    let restore_coordinator = std::sync::Arc::new(crate::state::restore::RestoreCoordinator::new(
        restore.clone(),
        layout.clone(),
        state_session.clone(),
        labels.clone(),
    ));

    // ── Shell settings load — before the boot stage plugin, which reads the UI language (ADR-0373) ──
    // ★Only reads here★: loading creates or changes no files or folders; writes open after setup's `enable_writes`. The single-instance gate decides inside build, so changing the disk here would let a second instance that is about to exit change it too.
    let settings = std::sync::Arc::new(crate::settings::SettingsService::load_from_dir(
        &crate::discovery::DataLayout::resolve().shell_config_dir(),
    ));

    let mut builder = tauri::Builder::default();
    // The single-instance plugin is registered first (plugin convention), always (ADR-0029).
    // ★"Global single" = the bundle identifier★: the plugin names the Windows mutex `{identifier}-sim` (tauri-plugin-single-instance 2.4.2 `platform_impl/windows.rs:67`), so two builds sharing the identifier kill each other. The dev build splits the identifier through the `src-tauri/tauri.dev.conf.json` overlay and coexists with the release. ★Do not unify the two identifiers★.
    // A second run is a person running the app again — the same "press" branch as a tray press. (TRD S26-window-chrome §6-2)
    builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
        crate::tray::actions::show_main_ui(app, crate::layout::window_ctl::Activation::TrayClick);
    }));
    // ★Right after the single instance★: log init, shell run lock, state file decision and model fill happen after the gate and before any window (canonical = that module's head). (TRD S21-storage §6-5)
    builder = builder.plugin(crate::state::boot_plugin::init(
        crate::state::boot_plugin::Boot {
            layout: layout.clone(),
            labels: labels.clone(),
            session: state_session.clone(),
            restore: restore.clone(),
            settings: settings.clone(),
        },
    ));
    builder = builder.plugin(tauri_plugin_opener::init());
    // Native folder picker (adding a preset path) — the frontend PresetPalette "Add" calls it via `open({directory:true})`; permission is `dialog:allow-open` in default.json.
    // ★Not a plugin only for folder picking★: the command refusal box (`daemon_client/refusal.rs`) uses it through the Rust API, and `dialog()` panics without this registration — do not remove it even if folder picking goes.
    builder = builder.plugin(tauri_plugin_dialog::init());

    // ★Plugin registration ≠ activation★: default OFF, enabled only via the set_autostart command / tray toggle (writes the registry Run entry).
    builder = builder.plugin(tauri_plugin_autostart::init(
        tauri_plugin_autostart::MacosLauncher::LaunchAgent,
        Some(vec!["--hidden"]),
    ));

    // ADR-0102: ★LayoutState must be managed pre-build (in the builder), not in setup()★. A webview loads JS as soon as its window exists and invokes `list_tabs`; with the state unregistered the command fails and main has no recovery path (`window:tabs-updated` fires only on tab changes), so it sticks on the loading placeholder forever (this happened when registration was in setup() and Tauri created the settings window before setup()). Registered in the builder, the state exists before any window regardless of order inside setup(). `LayoutState::new()` needs no app handle or runtime, so this is possible.
    //   ★Do not move it to setup★. DaemonClient needs the tokio runtime, so it stays in setup (registered before ⑧, which creates windows).
    //   The model inside is swapped for the one the boot-stage plugin decided before the windows (⑥). (TRD S21-storage §6-5 ⑥)
    let setup_layout = layout.clone();
    builder = builder.manage(layout);
    builder = builder.manage(labels.clone());
    builder = builder.manage(crate::state::placement::DeferredMaximize::default());
    // The restore status, for the same reason (ADR-0102): a window's first `restore_status` pull sees the value boot stage ⑥ decided.
    builder = builder.manage(restore);
    // The coordinator, for the same reason (ADR-0102). The window port, notices and subscription source need an `AppHandle`, so they are plugged in at the end of setup; an accept before that is `INTERNAL` (copy kept, can answer again) and a refusal stands without the port (`RestoreCoordinator` docs).
    let setup_restore = restore_coordinator.clone();
    builder = builder.manage(restore_coordinator);
    // Windows built hidden that wait for the first-paint notice; must exist before any window is created (otherwise they show right away) and before the prewarm pool, which holds its handle (the app handle is plugged in at the start of setup).
    let first_paint = std::sync::Arc::new(crate::first_paint::FirstPaintGate::default());
    let spare_paint_gate =
        std::sync::Arc::new(crate::first_paint::SparePaintGate::new(first_paint.clone()));
    let setup_spare_paint_gate = spare_paint_gate.clone();
    builder = builder.manage(first_paint);
    // The prewarm window state (pool · exit marker), for the same reason (ADR-0102): the webview's first measurement reports and `window_role` see it. (TRD S21-slot-docking §3-9)
    builder = builder.manage(std::sync::Arc::new(crate::spare::SpareState::new(
        crate::spare::SpareSeams::new(spare_paint_gate),
    )));
    // The tab drop ledger and the window facts last emitted per window (ADR-0296), for the same reason (ADR-0102): the webview's `window_frame` pull on mount and the first position record see them. (TRD S21-slot-docking §3-4 · §3-8)
    builder = builder.manage(std::sync::Arc::new(crate::tab_drop::DropBook::default()));
    builder = builder.manage(std::sync::Arc::new(crate::tab_drop::FrameBook::default()));
    // The OS title version last applied per window — must exist before any tab notice arrives (titles arriving while it is absent are passed over unapplied). (TRD S26-window-chrome §4-4)
    builder = builder.manage(crate::chrome::ChromeState::default());

    // ── Shell settings + effective theme (TRD S21-storage §5-3 · §5-6) ─────────────────────────────
    // ★Managed in the builder for the same reason as LayoutState (ADR-0102)★: without the state at the webview's first `get_ui_settings` / `settings_get`, that window freezes on the defaults.
    // ★One per shell★: the lock that keeps the push order is inside it (human and LLM paths share the instance). The window theme is read from the screen state model above. (TRD S21-storage §5-6)
    let themes = std::sync::Arc::new(crate::theme::EffectiveThemes::new(
        settings.clone(),
        setup_layout.clone(),
    ));
    builder = builder.manage(settings.clone()).manage(themes.clone());
    // One `ui.language` hook — the human path's and the LLM path's settings writes queue on the same instance (ADR-0373).
    builder = builder.manage(std::sync::Arc::new(
        crate::ui_language::LanguageHook::default(),
    ));

    // ── Webview environment (TRD S21-storage §4) — all code that creates windows gets folders · arguments from this instance ──
    // ★Managed in the builder (same reason as LayoutState above — ADR-0102)★ — it must exist before the invoke that creates a popout.
    let webview_env = std::sync::Arc::new(crate::webview_env::WebviewEnv::resolve());
    let setup_webview_env = webview_env.clone();
    builder = builder.manage(webview_env);

    let exit_session = state_session.clone();
    // The marker that the start failed — setup ⑧ sets it and the exit event reads it (`run` below).
    let startup_failed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let setup_startup_failed = startup_failed.clone();
    builder
        .setup(move |app| {
            // The log was already opened by the boot-stage plugin.

            // The app handle the prewarm pool's first-paint door uses to hand confirm ③'s show to the main thread — before any window is created (⑧, prewarm build). (TRD S21-storage §6-5)
            setup_spare_paint_gate.attach(app.handle().clone());

            // ── Webview folder decision — before any window is created ─────────────────
            // After the single-instance gate, so a write check (probe) is fine. (TRD S21-storage §4)
            setup_webview_env.chosen_dir();

            // ── Boot stage ⑦: start the writer (not started under a guard — §6-5 ③) ─────────────────────
            state_session.start_saver();

            // Past the single-instance gate, so changing the disk is fine. The logger is up, so the logs loading collected (unwritable `settings.json`, folded values) go out here.
            settings.enable_writes();

            // The window theme handle — the command table, boot push and restore coordinator hold the same one. (TRD S21-storage §5-6)
            let theme_control =
                crate::commands::settings::theme_control(app.handle().clone(), themes.clone());

            // ── ADR-0026 step 2: native tray wiring ─────────────────────────────────────
            // ADR-0029: the app always has a tray (no mode gate).
            // ADR-0028: the single owned state of the daemon liveness push; it is managed first so build_tray's initial refresh goes through publish (duplicate blocking, suppression window).
            app.manage(tray::actions::LivenessState::default());

            // ── Output plane (ADR-0046 — stateless pass-through): OutputRouter + window Channel registry ──
            // ★Two shared Arcs★: router and registry, the same instance on both sides.
            // ★No mirror buffer★: remount or a new window re-replays the daemon ring in full (view-driven, ADR-0046).
            let router = std::sync::Arc::new(crate::output_router::OutputRouter::new());
            let registry: crate::output_channel::WindowChannelRegistry = Default::default();
            app.manage(router.clone());
            app.manage(registry.clone());

            // ── The leg of webview-owned commands (ADR-0155) ──
            // ★Created before DaemonClient★: it is passed along when the table is plugged in (the registration packet carries both layers at once), and the invoke handler receiving the boot report must see the same object. (TRD §6)
            let view_commands = std::sync::Arc::new(crate::view_commands::ViewCommandBridge::new(
                std::sync::Arc::new(crate::view_commands::TauriViewDispatch(app.handle().clone())),
            ));
            app.manage(view_commands.clone());

            // ── DaemonClient (single authority of the daemon WS connection) registration ──────────
            // A client owning a dedicated multithreaded runtime (setup is outside a tokio context — `DaemonClient::new_real_with_owned_runtime`).
            // ★App-startup connect is deferred to T6/connect★: only the cmd plane is wired here; the frontend/boot sequence calls connect/ensure.
            match crate::daemon_client::DaemonClient::new_real_with_owned_runtime(
                router.clone(),
                registry.clone(),
                app.handle().clone(),
                std::sync::Arc::new(LayoutAgentNames {
                    layout: setup_layout.clone(),
                    app: app.handle().clone(),
                }),
            ) {
                Ok(client) => {
                    let client = std::sync::Arc::new(client);
                    // ── Plug the command table into that client (ADR-0155 decisions 4·5) ──
                    // ★The order is the contract★: the table's spawn port holds this client, so the client must be up first; and the connection is not up yet, so the table is plugged in before the first envelope. If missed, commands the daemon delivered come back with a "no table" error.
                    // ★Passes the **same** state · router · issuer as human clicks★, or a window an LLM created would not be in the list a person sees (ADR-0035).
                    match app.try_state::<crate::layout::LayoutState>() {
                        Some(state) => client.install_command_table(
                            crate::layout::commands::make_table(
                                crate::commands::layout::command_ports(
                                    app.handle().clone(),
                                    state.inner().clone(),
                                    router.clone(),
                                    labels.clone(),
                                    client.clone(),
                                    settings.clone(),
                                    theme_control.clone(),
                                    setup_restore.clone(),
                                ),
                            ),
                            crate::layout::commands::CATALOG_VERSION,
                            view_commands.clone(),
                        ),
                        // Not reached while LayoutState is managed in the builder (ADR-0102); if it is, that registration is gone, so do not pass over it silently.
                        None => tracing::error!(
                            "LayoutState not registered — could not plug in the layout command table (commands the daemon delivers will fail)"
                        ),
                    }
                    app.manage(client);
                }
                Err(e) => {
                    tracing::warn!("failed to create the DaemonClient runtime (no daemon commands, app continues): {e}")
                }
            }

            // ── Boot stages ⑧ ⑨: create the static window (main) at the saved position · restored popout windows · theme once ──
            // The first window is created here (`"create": false` in the config). With `--hidden`, main is not shown even at first paint, and popouts are kept hidden by the hide below withdrawing their first-paint wait (F13). (TRD S21-storage §4 · §6-5)
            if crate::state::placement::restore_windows(
                app.handle(),
                &setup_webview_env,
                hidden,
                &setup_layout,
                &theme_control,
            )
            .is_err()
            {
                // ADR-0283
                // If main cannot be created the app ends: an exit request, not a panic, so the exit event (`RunEvent::Exit`, `run` below) does the writer `Final` and the shell run lock release, and the set marker decides the exit code. Setup ends here since the remaining stages are of no use to an app about to end.
                setup_startup_failed.store(true, std::sync::atomic::Ordering::SeqCst);
                app.handle().exit(STARTUP_FAILED_EXIT_CODE);
                return Ok(());
            }
            // Apply the OS title once to each built window, after ⑨ removed the popouts it could not open from the model. Without it the title stays at the build-time "Engram" until the next tab change. (TRD S26-window-chrome §4-4 ③)
            if let Err(e) = crate::layout::apply::sync_titles(
                &setup_layout,
                &crate::commands::layout::TauriEvents { app: app.handle() },
            ) {
                tracing::warn!("could not apply the window titles after boot (applied on the next tab change): {e}");
            }

            // ── Boot stage ⑩: recompute the derived tables (router · usage interest) once with the final window set ──
            // The boot-stage plugin runs before this client and could not call it, and it must follow ⑨ (popout removal). Without it, agent output of restored slots goes to no window until the next layout change.
            if let Some(client) = app.try_state::<std::sync::Arc<crate::daemon_client::DaemonClient>>() {
                match setup_layout.0.lock() {
                    Ok(mgr) => crate::layout::SubscriptionSync::resync(
                        &crate::commands::layout::RouterSubs {
                            router: &router,
                            client: &client,
                        },
                        &mgr,
                    ),
                    Err(_) => {
                        tracing::error!("the layout lock is poisoned, so the post-boot subscription recompute could not run")
                    }
                }
            }
            // ── Restore coordinator port (TRD S21-storage §6-7) ──────────────────────────────────────────
            // ★Plugged in after ⑨ · ⑩★: ⑨ (`open_restored_popouts`) is boot-only; an accept committed before it would fail recreating the window the accept already created, remove that popout from the model and orphan the OS window (docs in `state::placement`).
            setup_restore.attach(crate::state::restore::RestorePorts {
                windows: std::sync::Arc::new(crate::state::placement::TauriRestoreWindows::new(
                    app.handle().clone(),
                )),
                events: std::sync::Arc::new(crate::commands::layout::OwnedEvents {
                    app: app.handle().clone(),
                }),
                subs: std::sync::Arc::new(crate::commands::layout::AppSubscriptions {
                    app: app.handle().clone(),
                }),
                themes: theme_control,
            });
            // TODO(T6/connect): set up the connection automatically at boot via DaemonClient.ensure()/connect().
            if let Err(e) = tray::build_tray(app) {
                tracing::warn!("failed to create the tray (the app continues): {e}");
            }
            // ADR-0028: daemon liveness cycle (external changes reach the tray and emit too); after build_tray settles the initial icon it pushes only changes.
            tray::spawn_daemon_observer(&app.handle().clone());

            // ── Spare window — build one in advance and keep it waiting hidden ──
            // ★Built on a worker thread★: the event loop creates windows after this setup returns. The spare is written to the pool before it is built so the `--hidden` hide and the tray below do not see it, and it comes after main (⑧) so the first window does not change.
            // ADR-0295 · ADR-0299 (TRD S21-slot-docking §3-9)
            crate::commands::spare::spawn_spare_build(app.handle());

            // `--hidden`: main was not raised at first paint (⑧); here the raise ⑨ gave the popouts is withdrawn (F13 — otherwise the first-paint notice or deadline shows only the popouts while main stays in the tray) and those windows leave the usage interest — one hide path (ADR-0229). The first-paint show decides on the main thread right before showing and this setup is on the main thread too, so it runs only after this withdrawal.
            if hidden {
                crate::tray::actions::hide_main_ui(app.handle());
            }
            Ok(())
        })
        // ADR-0026: main X (WM_CLOSE) = hide (stays in the tray); the real exit is only the tray's exit item (`app.exit(0)`). CloseRequested is a Rust-side event, so the JS capability `core:window:allow-close` is not needed.
        .on_window_event(move |window, event| {
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    let label = window.label();
                    // ADR-0229: there is one hide path — hiding the window separately here would split which windows each entry hides.
                    if label == crate::layout::MAIN_WINDOW_LABEL {
                        api.prevent_close();
                        crate::tray::actions::hide_main_ui(window.app_handle());
                    } else if crate::commands::popout::is_popup_label(label) {
                        // A popout removes the model first on a close request and puts a tombstone on the label, blocking a confirm that would attach to a closing window. ★Premise: a popout's close request is never reverted★ — cancelling happens only in the main branch above, the frontend has no close-request listener, and the runtime closes the window as soon as this callback returns (tauri-runtime-wry 2.11.3 `on_close_requested` → `on_window_close`). If a listener or close cancel appears, the removed window survives; then change to "closing mark + release if blocked". Check (every `/engram-qa`, repo root): `rg -i "close.?requested" src/` → 0 lines · `rg "prevent[_]close" src-tauri/src/` → 1 line (the main branch above). (TRD S21-slot-docking 5.8th edition §1-2)
                        let app = window.app_handle();
                        // If one is missing (extreme init failure), skip — same as the `Destroyed` branch below.
                        if let (Some(state), Some(router), Some(client)) = (
                            app.try_state::<crate::layout::LayoutState>(),
                            app.try_state::<std::sync::Arc<crate::output_router::OutputRouter>>(),
                            app.try_state::<std::sync::Arc<crate::daemon_client::DaemonClient>>(),
                        ) {
                            crate::commands::popout::close_requested_popup_window(
                                label, &state, &router, &client,
                            );
                        }
                    }
                }
                // ★Popup window Destroyed cleanup (lifetime/leak critical)★: runs when a popup is actually destroyed (normal close or program destroy); main is never a target (it is only hidden above). A forced process kill takes all state with it, so this path is not taken (accepted). ADR-0046.
                tauri::WindowEvent::Destroyed => {
                    let label = window.label().to_string();
                    // Outside the popout/state conditions below, so even if those filter the window out its title ledger entry does not remain (TRD S26-window-chrome §4-4).
                    crate::chrome::forget(window.app_handle(), &label);
                    if crate::commands::popout::is_popup_label(&label) {
                        let app = window.app_handle();
                        crate::state::placement::forget_deferred_maximize(app, &label);
                        crate::first_paint::cancel_hold(app, &label);
                        crate::commands::tab_drop::forget_window_frame(app, &label);
                        // The cleanup frees the window number inside one lock with the tombstone and subscription cleanup. If one is missing (extreme init failure) the cleanup is skipped and only the number is freed, or it would stay taken until the run ends (TRD S26-window-chrome §4-1).
                        if let (Some(state), Some(router), Some(registry), Some(client)) = (
                            app.try_state::<crate::layout::LayoutState>(),
                            app.try_state::<std::sync::Arc<crate::output_router::OutputRouter>>(),
                            app.try_state::<crate::output_channel::WindowChannelRegistry>(),
                            app.try_state::<std::sync::Arc<crate::daemon_client::DaemonClient>>(),
                        ) {
                            crate::commands::popout::cleanup_popup_window(
                                &app, &label, &state, &router, &registry, &client,
                            );
                        } else if let Some(state) = app.try_state::<crate::layout::LayoutState>() {
                            crate::layout::release_window_number(&state, &label);
                        }
                    }
                    // Prewarm pool: a pool window is removed and rebuilt, and an adopted window waiting to be shown completes that rebuild (same completion as the handed-over show and the safety deadline — once per label). (TRD S21-slot-docking §3-9)
                    crate::commands::spare::on_spare_window_destroyed(window.app_handle(), &label);
                }
                // Window position record — reads the getters again instead of the event value (minimized/maximized are needed too). Spare-pool windows are outside the model and not written.
                // ADR-0295 (TRD S21-storage §6-3; TRD S21-slot-docking §3-9)
                tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
                    if !crate::commands::spare::is_spare_window(window.app_handle(), window.label()) {
                        crate::state::placement::record(window);
                    }
                    // Applies the recorded OS minimum size to a window that returned to normal size from maximized/minimized (not applied while maximized/minimized; reason = the head of the `crate::chrome` ledger). Prewarm windows report too, so they are not excluded. (TRD S26-window-chrome §7-7)
                    if matches!(event, tauri::WindowEvent::Resized(_)) {
                        crate::chrome::on_resized(window);
                    }
                }
                // The foreground field of the window facts (title bar dimming): pushed with the event's value. When another of our windows comes to the front, both its `Focused(true)` and the old one's `Focused(false)` arrive and each pushes. Windows outside the model (prewarm) are skipped by the pushing side. (TRD S26-window-chrome §6-5)
                tauri::WindowEvent::Focused(focused) => {
                    crate::commands::tab_drop::push_window_focus(window, *focused);
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::discover_daemon,
            commands::daemon_start,
            commands::daemon_shutdown,
            commands::daemon_status,
            commands::read_daemon_info,
            commands::daemon_connect,
            commands::daemon_ensure,
            commands::daemon_close,
            commands::daemon_connection_state,
            commands::forward_daemon_command,
            commands::get_usage_snapshot,
            commands::show_main_ui,
            commands::hide_main_ui,
            commands::quit_app,
            commands::set_autostart,
            commands::get_autostart,
            commands::create_tab,
            commands::create_window,
            commands::switch_tab,
            // Restoring the original selection of a tab drag (webview → shell) — not a bus command. (TRD S21-slot-docking §4-6)
            commands::switch_tab_if,
            commands::close_tab,
            commands::split_slot,
            commands::set_split_ratio,
            commands::close_slot,
            commands::focus_slot,
            commands::rename_tab,
            commands::assign_agent,
            commands::set_slot_content,
            commands::set_usage_slot,
            commands::spawn_into,
            commands::get_view,
            commands::list_tabs,
            commands::list_windows,
            commands::resolve_spatial,
            // Measurement reports (webview → shell) — not bus commands (ADR-0227).
            commands::report_window_canvas,
            commands::report_ui_metrics,
            // First-paint notice (webview → shell) — not a bus command (`first_paint`).
            commands::webview_painted,
            // The calling window's role (spare pool) — wiring inside the webview, not a bus command. (TRD S21-slot-docking §3-9)
            commands::window_role,
            // Tab drop · window facts (webview → shell, ADR-0296) — not bus commands (end of a mouse drop, wiring inside the webview — TRD S21-slot-docking §3-6). The LLM command for the same action is T-65 `tab.dock`.
            commands::tab_drop::tab_drop,
            commands::tab_drop::window_frame,
            // Window state (human path — web buttons · registry) — the same service as the bus `window.*`. ★No separate window-close-only invoke★: main's close is hiding all windows (§5-4), so two close paths would split that decision. (TRD S26-window-chrome §6-4)
            commands::window_ctl::window_control,
            // Minimum window size report (webview → shell) — a measurement report, not a bus command (ADR-0227). (TRD S26-window-chrome §7-7)
            commands::window_ctl::report_window_min_width,
            // Maximize button rectangle report (webview → shell) — places the snap layout overlay there; a measurement report, not a bus command (ADR-0227). ★Behind the shell feature `snap-overlay` (default off)★. (TRD S26-window-chrome §8-4)
            #[cfg(feature = "snap-overlay")]
            commands::snap_overlay::report_max_button_rect,
            // Boot query — the pushing side (`window.setTheme` · writing `theme.default` · boot · restore accept) is separate
            //   (`commands/settings.rs` 「why there are two places that read the per-window theme」).
            commands::get_ui_settings,
            // Shell settings — the same service as the bus `settings.*` (ADR-0081 decision 3).
            commands::settings_get,
            commands::settings_set,
            commands::settings_reset,
            commands::settings_schema,
            // Crash copy — the same coordinator as the bus `restore.*`. (TRD S21-storage §6-7)
            commands::restore_status,
            commands::restore_answer,
            // Webview-owned commands (ADR-0155) — the pair of boot report and outcome collection (`commands/view_bus.rs`).
            commands::report_view_commands,
            commands::report_command_outcome,
            commands::agent_spawn,
            commands::agent_kill,
            commands::agent_interrupt,
            commands::agent_write_stdin,
            commands::agent_resize,
            commands::set_envelope_format,
            commands::subscribe_output,
            commands::request_replay,
            commands::move_slot_to_window,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        // ADR-0029: no in-proc agents, so nothing to reap at exit (the daemon handles its own agents). Here: the normal exit write of the screen state, releasing the shell run lock, and deciding the exit code if the start failed — the tray's exit item (`app.exit(0)`) also emits this event. (TRD S21-storage §6-6)
        .run(move |handle, event| {
            // While exiting, a vanished spare window is not rebuilt. Nothing blocks exit (`rg "prevent[_]exit" src-tauri/src/` → 0 lines), so a request is the exit — if a blocking path appears, move this marker to `Exit` only. (TRD S21-slot-docking §3-9)
            if let tauri::RunEvent::ExitRequested { .. } = event {
                crate::commands::spare::on_app_exiting(handle);
            }
            if let tauri::RunEvent::Exit = event {
                crate::commands::spare::on_app_exiting(handle);
                exit_session.shutdown();
                // An exit request's code does not become the process exit code (tauri-runtime-wry 2.11.3 ends it with `ControlFlow::Exit`, tao 0.35 = code 0), so exit directly here. Plugins' exit handling (releasing the single-instance mutex) ran before this callback; the only thing skipped is Tauri's later `cleanup_before_exit` (tray icon removal, hiding windows), and this path ends setup before creating the tray and has no main, so there is nothing to remove.
                if startup_failed.load(std::sync::atomic::Ordering::SeqCst) {
                    std::process::exit(STARTUP_FAILED_EXIT_CODE);
                }
            }
        });
}
