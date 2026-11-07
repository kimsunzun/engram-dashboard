//! The Tauri adapter for webview-owned commands — one boot report and one outcome retrieval.
//!
//! ★There is no logic in this file★: filtering, correlation and closing are owned by `crate::view_commands`, and what remains here is
//! the translation into the Tauri world (where to get the window label · what argument receives the outcome) and just **whether to push a delta to the daemon**
//! (the same division of labor as `commands/settings.rs` · `commands/layout.rs`).
//!
//! ## ★Why the delta is pushed here★
//! The shell's registration goes out **once right after the handshake** (`register_commands` in `daemon_client::connection`).
//! The webview's report has no order relation to that moment at all — if the window script comes up late, the registration packet of that connection
//! carries **no screen name at all**. Left as is, screen commands are entirely `UNKNOWN_COMMAND` until the next reconnect.
//! So `UpdateCommands` is sent only for reports where the list actually changed (TRD §3-7 clause 3).
// ADR-0155

use std::sync::Arc;

use engram_dashboard_protocol::{AgentCommand, RequestId};
use tauri::{AppHandle, Manager, State, Window};

use crate::daemon_client::protocol_state::{self, CommandFailure};
use crate::daemon_client::refusal::RefusalSite;
use crate::daemon_client::DaemonClient;
use crate::view_commands::{ViewCommandBridge, ViewCommandDecl};
use engram_dashboard_command::{CommandError, ErrorCode};

/// One window announces its own command list at boot — once per window (`src/commands/viewCommandBridge.ts`).
///
/// ★It does not take the window label as an argument★: if the webview states it itself, one mistyped label sends an envelope
/// to someone else's window. The [`Window`] that Tauri injects is the sole authority of that value.
/// ★It is the **window** label, not the webview label★ — it must equal the value the webview side subscribes with (`getCurrentWindow().label`)
/// for `emit_to` to hit that listener. The two are the same string today (one webview per window), but we choose the one that
/// **points at** the same thing.
/// The return is always `Ok` — a reserved name being dropped is the shell's policy, not a failure of the reporter (reason =
/// the `view_commands` module header), and the log carries that fact.
#[tauri::command]
pub async fn report_view_commands(
    window: Window,
    app: AppHandle,
    bridge: State<'_, Arc<ViewCommandBridge>>,
    commands: Vec<ViewCommandDecl>,
) -> Result<(), String> {
    let label = window.label().to_string();
    let mut refused_by_daemon = None;
    // ★The bridge runs the state change and its delta send **inside one gate**★ — if called separately here, the reports of two windows
    //   overtake each other and the daemon is left holding old names (reason = `outbound` of `ViewCommandBridge`).
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
            "웹뷰 명령 일부를 등록에서 뺐다(셸이 답하는 이름이거나 설명·표식이 없다)"
        );
    }
    if !outcome.changed() {
        tracing::debug!(window = %label, "웹뷰 명령 보고 — 광고 명단 변화 없음");
    }
    Ok(())
}

/// Puts only the changed share onto the daemon's registry — ★the only place that calls it is the one above★ (inside the ordering gate).
/// If the daemon rejected with `Error`, it returns that message — the box is shown by the caller after releasing the gate.
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
        "웹뷰 명령 명단 갱신"
    );
    // ★If there is no client, end quietly★ — in an app whose runtime creation failed (the Err branch in `lib.rs`) there is no daemon
    //   command at all. The list is already in the bridge, so the registration packet of the next connection carries it.
    let Some(client) = app
        .try_state::<Arc<DaemonClient>>()
        .map(|c| c.inner().clone())
    else {
        return None;
    };
    // ADR-0272: wrap in the raw field. A failure leaves only a warn, like a shell-local failure (the `Local` branch below), and shows no box
    //   (boxes are for daemon origin only — ADR-0281).
    let added = match protocol_state::decls_to_wire(&added) {
        Ok(added) => added,
        Err(e) => {
            tracing::error!(
                window = %label,
                "웹뷰 명령 차분을 원문 칸으로 싸지 못했다 — 이 차분은 데몬에 안 나간다: {e}"
            );
            return None;
        }
    };
    let cmd = AgentCommand::UpdateCommands {
        // ★The daemon does not use this field★ — the registry owner is derived from the connection that packet came on. Write the **same
        //   advertisement string** as the registration packet so that the two packets look like the same shell's in the daemon log.
        owner: crate::daemon_client::inbound::shell_owner_advert(),
        added,
        removed,
        request_id: RequestId::new(),
    };
    // ★Wait for the reply but do not raise failure upward★: the report itself succeeded (it is in the bridge). A shell-local failure is
    //   only 「이 연결에서는 아직 안 보인다」 (not visible on this connection yet), so the full registration of the next reconnect sends that list again (retry). If that
    //   registration is also rejected, the registration site leaves a warn and shows a rejection box. A daemon rejection is different —
    //   the bridge holds the rejected name without reverting it, so if the declaration stays the same the next full registration may be rejected too, so the daemon message is
    //   passed on and raised as a rejection box (ADR-0281). Either way leave a warn — if registration fails, screen commands are not
    //   called and that cause is left nowhere.
    match client.send_command_with_origin(cmd).await {
        Ok(_) => {
            tracing::info!(window = %label, "웹뷰 명령 차분 등록 완료");
            None
        }
        Err(CommandFailure::Daemon { message }) => {
            tracing::warn!(
                window = %label,
                "웹뷰 명령 차분 등록 거절(셸이 그 이름을 쥔 채라 선언을 고치지 않으면 다음 재연결의 전량 등록도 거절될 수 있다): {message}"
            );
            Some(message)
        }
        // No box — boxes are shown only for daemon origin (ADR-0281).
        Err(CommandFailure::Local(message)) => {
            tracing::warn!(
                window = %label,
                "웹뷰 명령 차분 등록 실패(다음 재연결의 전량 등록이 해소한다): {message}"
            );
            None
        }
    }
}

/// The webview finishes one command and returns the outcome.
///
/// ★Which window answered is given by [`Window`] — it is not taken as an argument★: if opened by a correlation key alone, a window that did not receive
/// the envelope could end someone else's round trip (the caller receives a forged outcome, the real window's side effects still happen and
/// only its answer is bounced). The bridge does the check (`ViewCommandBridge::settle`).
///
/// ★The webview cannot pick the error kind (a known limit)★ — both an argument mistake and a real failure go out as `INTERNAL` + message.
/// To tell them apart the webview registry would have to throw typed codes, and that is not this adapter's decision (the advertised error
/// set also states that fact as is — `ViewCommandHelp::to_catalog_item`).
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
        // There is no place to attach the reply = the deadline was passed · a second reply came for the same envelope · **a window that did not receive
        //   the envelope answered**. All three are signals a human should look at.
        tracing::warn!(window = %window.label(), "웹뷰 명령 결말을 상관시키지 못했다: {detail}");
        return Err(detail);
    }
    Ok(())
}
