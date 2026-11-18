//! The Tauri adapter for webview-owned commands — one boot report and one outcome retrieval. No logic in this file: filtering, correlation and closing are `crate::view_commands`; what remains is translation into the Tauri world (where to get the window label · what argument receives the outcome) and just whether to push a delta to the daemon (same division as `commands/settings.rs` · `commands/layout.rs`).
//! Why the delta is pushed here: the shell's registration goes out once right after the handshake (`register_commands` in `daemon_client::connection`), and the webview's report has no order relation to that moment — a window script coming up late leaves that connection's registration packet with no screen name at all, so screen commands would all be `UNKNOWN_COMMAND` until the next reconnect. So `UpdateCommands` is sent only for reports where the list actually changed (TRD §3-7 clause 3).
// ADR-0155

use std::sync::Arc;

use engram_dashboard_protocol::{AgentCommand, RequestId};
use tauri::{AppHandle, Manager, State, Window};

use crate::daemon_client::protocol_state::{self, CommandFailure};
use crate::daemon_client::refusal::RefusalSite;
use crate::daemon_client::DaemonClient;
use crate::view_commands::{ViewCommandBridge, ViewCommandDecl};
use engram_dashboard_command::{CommandError, ErrorCode};

/// One window announces its own command list at boot — once per window (`src/commands/viewCommandBridge.ts`). It does not take the window label as an argument: a webview stating it itself could, with one mistyped label, send an envelope to someone else's window; the injected [`Window`] is the sole authority. It is the window label, not the webview label — it must equal what the webview subscribes with (`getCurrentWindow().label`) for `emit_to` to hit that listener; the two are the same string today (one webview per window), but the one that points at the same thing is chosen. Always `Ok` — a dropped reserved name is the shell's policy, not a failure of the reporter (reason = the `view_commands` module header), and the log carries it.
#[tauri::command]
pub async fn report_view_commands(
    window: Window,
    app: AppHandle,
    bridge: State<'_, Arc<ViewCommandBridge>>,
    commands: Vec<ViewCommandDecl>,
) -> Result<(), String> {
    let label = window.label().to_string();
    let mut refused_by_daemon = None;
    // The bridge runs the state change and its delta send inside one gate — called separately here, two windows' reports would overtake each other and leave the daemon holding old names (reason = `outbound` of `ViewCommandBridge`).
    let outcome = bridge
        .report_and_push(&label, commands, |added, removed| {
            let label = label.clone();
            let app = app.clone();
            let refused_by_daemon = &mut refused_by_daemon;
            async move { *refused_by_daemon = push_delta(&app, &label, added, removed).await }
        })
        .await;
    // ADR-0006: the box is called after releasing that gate (the `outbound` lock).
    if let Some(message) = refused_by_daemon {
        if let Some(client) = app.try_state::<Arc<DaemonClient>>() {
            client
                .refusal_alerts()
                .surface(RefusalSite::Update, &message);
        }
    }
    if !outcome.refused.is_empty() {
        tracing::info!(
            window = %label,
            names = ?outcome.refused,
            "dropped some webview commands from registration (names the shell answers, or missing description or marker)"
        );
    }
    if !outcome.changed() {
        tracing::debug!(window = %label, "webview command report — advertised list unchanged");
    }
    Ok(())
}

/// Puts only the changed part onto the daemon's registry — the only caller is the one above (inside the ordering gate). If the daemon rejected with `Error` it returns that message — the caller shows the box after releasing the gate.
async fn push_delta(
    app: &AppHandle,
    label: &str,
    added: Vec<engram_dashboard_command::CommandDecl>,
    removed: Vec<String>,
) -> Option<String> {
    tracing::info!(
        window = %label,
        added = added.len(),
        removed = removed.len(),
        "webview command list update"
    );
    // No client → end quietly: an app whose runtime creation failed (the Err branch in `lib.rs`) has no daemon command at all; the list is already in the bridge, so the next connection's registration packet carries it.
    let Some(client) = app
        .try_state::<Arc<DaemonClient>>()
        .map(|c| c.inner().clone())
    else {
        return None;
    };
    // ADR-0272: wrap in the raw field. A failure leaves only a warn like a shell-local failure (the `Local` branch below) and shows no box (boxes are for daemon origin only — ADR-0281).
    let added = match protocol_state::decls_to_wire(&added) {
        Ok(added) => added,
        Err(e) => {
            tracing::error!(
                window = %label,
                "could not wrap the webview command diff into a raw-text field — this diff is not sent to the daemon: {e}"
            );
            return None;
        }
    };
    let cmd = AgentCommand::UpdateCommands {
        // The daemon does not use this field (the registry owner is derived from the connection the packet came on) — write the same advertisement string as the registration packet so both packets look like the same shell's in the daemon log.
        owner: crate::daemon_client::inbound::shell_owner_advert(),
        added,
        removed,
        request_id: RequestId::new(),
    };
    // Wait for the reply but do not raise failure upward: the report itself succeeded (it is in the bridge). A shell-local failure is only "not visible on this connection yet", so the next reconnect's full registration sends the list again (retry); if that registration is rejected too, the registration site leaves a warn and shows a rejection box. A daemon rejection differs — the bridge holds the rejected name without reverting it, so with the declaration unchanged the next full registration may be rejected too; the daemon message is passed on and raised as a rejection box (ADR-0281). Either way leave a warn — otherwise a failed registration (screen commands not called) leaves its cause nowhere.
    match client.send_command_with_origin(cmd).await {
        Ok(_) => {
            tracing::info!(window = %label, "webview command diff registered");
            None
        }
        Err(CommandFailure::Daemon { message }) => {
            tracing::warn!(
                window = %label,
                "webview command diff registration refused (the shell still holds that name, so unless the declaration is fixed the next reconnect's full registration may be refused too): {message}"
            );
            Some(message)
        }
        // No box — boxes are shown only for daemon origin (ADR-0281).
        Err(CommandFailure::Local(message)) => {
            tracing::warn!(
                window = %label,
                "webview command diff registration failed (the next reconnect's full registration resolves it): {message}"
            );
            None
        }
    }
}

/// The webview finishes one command and returns the outcome. Which window answered is given by [`Window`], not an argument: opened by a correlation key alone, a window that did not receive the envelope could end someone else's round trip (the caller gets a forged outcome, the real window's side effects still happen and only its answer is bounced); the bridge does the check (`ViewCommandBridge::settle`).
/// The webview cannot pick the error kind (known limit): an argument mistake and a real failure both go out as `INTERNAL` + message; telling them apart would need typed codes thrown by the webview registry, which is not this adapter's decision (the advertised error set states that as is — `ViewCommandHelp::to_catalog_item`).
#[tauri::command]
pub fn report_command_outcome(
    window: Window,
    bridge: State<'_, Arc<ViewCommandBridge>>,
    request_id: String,
    ok: Option<serde_json::Value>,
    error: Option<String>,
) -> Result<(), String> {
    let outcome = match error {
        Some(message) => Err(CommandError::of(ErrorCode::Internal, message)),
        None => Ok(ok.unwrap_or(serde_json::Value::Null)),
    };
    if let Err(detail) = bridge.settle(window.label(), &request_id, outcome) {
        // No place to attach the reply = the deadline passed · a second reply for the same envelope · a window that did not receive the envelope answered. All three are signals a human should look at.
        tracing::warn!(window = %window.label(), "could not correlate the webview command outcome: {detail}");
        return Err(detail);
    }
    Ok(())
}
