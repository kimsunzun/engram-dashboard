//! Agent command invoke handlers — the request/reply plane (S14 module ① T6a, ADR-0036). When the frontend (or the §5 LLM) calls `invoke('agent_spawn'|…)`, this builds an `AgentCommand`, sends it through `DaemonClient::send_command` and returns the daemon reply (matched by request_id) as a frontend-friendly Result. Connection semantics live in `DaemonClient`; this layer is only a thin builder + injection.
//! request_id is set here: each handler creates a new key with `RequestId::new()` and puts it in the command; `send_command` matches the reply by that key (the "request_id source" of spike §9 G1 is this layer). Idempotency semantics would keep the same key on retry, but a one-shot invoke needs a new key each time (on a disconnect the caller re-calls = a new key). Preventing writeStdin duplicates is the daemon-side dedup table's job (ids.rs RequestId comment).

use std::sync::Arc;

use engram_dashboard_protocol::{
    AgentCommand, AgentEvent, AgentId, EnvelopeFormat, ProfileId, RequestId,
};
use tauri::State;
use uuid::Uuid;

use crate::daemon_client::DaemonClient;
use crate::output_channel::WindowChannelRegistry;

fn parse_uuid(s: &str, what: &str) -> Result<Uuid, String> {
    Uuid::parse_str(s).map_err(|e| format!("failed to parse {what} UUID: {e}"))
}

type CmdResult = Result<AgentEvent, String>;

// Reply kind: the daemon `connection_core.rs` `Spawn{profile_id}` dispatch answers with Ack (only SpawnByCwd/SpawnProfile answer Spawned with AgentInfo).
#[tauri::command]
pub async fn agent_spawn(
    client: State<'_, Arc<DaemonClient>>,
    profile_id: String,
) -> Result<(), String> {
    let profile_id: ProfileId = parse_uuid(&profile_id, "profile_id")?;
    let cmd = AgentCommand::Spawn {
        profile_id,
        request_id: RequestId::new(),
    };
    expect_ack_or_spawned(client.send_command(cmd).await)
}

#[tauri::command]
pub async fn agent_kill(
    client: State<'_, Arc<DaemonClient>>,
    agent_id: String,
) -> Result<(), String> {
    let agent_id: AgentId = parse_uuid(&agent_id, "agent_id")?;
    let cmd = AgentCommand::Kill {
        agent_id,
        request_id: RequestId::new(),
    };
    expect_ack(client.send_command(cmd).await)
}

// Interrupts only the work in progress; the process survives. Terminal mode (PTY) is rejected — the terminal receives keys directly.
// ADR-0245
#[tauri::command]
pub async fn agent_interrupt(
    client: State<'_, Arc<DaemonClient>>,
    agent_id: String,
) -> Result<(), String> {
    let agent_id: AgentId = parse_uuid(&agent_id, "agent_id")?;
    let cmd = AgentCommand::Interrupt {
        agent_id,
        request_id: RequestId::new(),
    };
    expect_ack(client.send_command(cmd).await)
}

#[tauri::command]
pub async fn agent_write_stdin(
    client: State<'_, Arc<DaemonClient>>,
    agent_id: String,
    data: Vec<u8>,
) -> Result<(), String> {
    let agent_id: AgentId = parse_uuid(&agent_id, "agent_id")?;
    let cmd = AgentCommand::WriteStdin {
        agent_id,
        data,
        request_id: RequestId::new(),
    };
    expect_ack(client.send_command(cmd).await)
}

// PTY resize. `Resize` has no request_id on the wire (the daemon sends no reply), so it is not a reply-matching target of `send_command` — **fire-and-forget** (`send_fire_and_forget`) is right; a reply-expecting path would hang forever. It only enqueues and waits for no ack (an unapplied resize is just a screen size mismatch, harmless). Not connected → `DaemonClient` quietly no-ops; Resize is a one-shot command, not a subscription delta, so it is not re-synced on connect (the next resize input updates the dimensions). `Ok(())` means "a send was attempted", not "daemon application confirmed".
#[tauri::command]
pub async fn agent_resize(
    client: State<'_, Arc<DaemonClient>>,
    agent_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let agent_id: AgentId = parse_uuid(&agent_id, "agent_id")?;
    client.send_fire_and_forget(AgentCommand::Resize {
        agent_id,
        cols,
        rows,
        viewport_id: None,
    });
    Ok(())
}

// Global envelope format switch (ADR-0096) — a steering/invoke control surface. An operator (or the §5 LLM / resident orchestrator) calls `invoke('set_envelope_format', { format: 'colon' | 'xml' })` to change the daemon-global envelope format state; afterwards every A→B message envelope renders in that format (colon = `{sender}: {body}` default, xml = `<message from="{sender}">{body}</message>`).
// src-tauri = client shell (ADR-0029): state ownership and assembly belong to the daemon (the process owning AgentManager). Not exposed on the worker MCP channel — a managed agent must not change a team-global format (ADR-0096 decision 3 · ADR-0094 least privilege). This invoke surface is the official active path; a human trigger (shortcut/UI button) is a later option (ADR-0096 decision 5 — this invoke is temporarily called and verified with `cdp.mjs eval`).
// `EnvelopeFormat` is serde lowercase, so the invoke JSON `{format:"xml"}` comes in as is (wire enum contract, ADR-0096).
// ADR-0096
#[tauri::command]
pub async fn set_envelope_format(
    client: State<'_, Arc<DaemonClient>>,
    format: EnvelopeFormat,
) -> Result<(), String> {
    let cmd = AgentCommand::SetEnvelopeFormat {
        format,
        request_id: RequestId::new(),
    };
    expect_ack(client.send_command(cmd).await)
}

// Output Channel registration (ADR-0046 — registration only, no replay trigger). The frontend calls `invoke('subscribe_output', { channel })` on window mount; it puts that window's output Channel into the window_label → Channel registry, and the connection task fans that window's agent output out to it by the routing table (the agent_id tag is built into the frame).
// Automatic window label injection: taking `tauri::Window` makes Tauri inject the calling webview, so the label comes from `window.label()` (the frontend need not pass it and cannot forge it); the Channel is also bound to the calling webview by birth (spike §7), so the label↔Channel pair is always consistent.
// Raw bytes (spike §7): the registry type is `Channel<tauri::ipc::Response>` so the connection task sends `Response::new(bytes)` (avoiding the JSON serialization trap of `Channel<Vec<u8>>`).
// ADR-0046: with the mirror buffer removed, `subscribe_output` does Channel registration only (the old register-then-replay-immediately is deleted); the view triggers replay separately on mount with `request_replay` — registration (per window) and replay (per view/agent) are separate so multiple views of one window each know their own replay boundary through the gen fence.
#[tauri::command]
pub fn subscribe_output(
    registry: State<'_, WindowChannelRegistry>,
    window: tauri::Window,
    channel: tauri::ipc::Channel<tauri::ipc::Response>,
) -> Result<(), String> {
    let label = window.label().to_string();
    // ADR-0006: the registry is a std Mutex — insert is synchronous, 0 awaits while holding the lock. Re-registering the same label (window reload) overwrites (the old Channel is dropped — harmless, that webview is dead). Register even if poisoned (ADR-0231): the old `Err` return left that window never receiving output (rationale = the `output_channel` module header).
    crate::output_channel::register_window(&registry, label, channel);
    Ok(())
}

// View-driven replay numbering (ADR-0046 M1 — request_replay). A view (slot) calls it on mount/remount; it triggers a full re-replay of that agent's daemon ring as single-flight and returns the assigned `gen`. The view does the sort+dedup flush only on success markers whose gen is at or above the gen it received (gen fence — blocks an early flush by someone else's / an old generation's replay).
// BLOCK-1 made universal (ADR-0046): this is the only path that sends wire `Subscribe{after_seq:None}` (everything); layout sends only Unsubscribe (cleanup). Not connected → Err (the frontend re-requests on the connected transition — M2).
#[tauri::command]
pub async fn request_replay(
    client: State<'_, Arc<DaemonClient>>,
    agent_id: String,
) -> Result<u64, String> {
    let agent_id: AgentId = parse_uuid(&agent_id, "agent_id")?;
    client.request_replay(agent_id).await
}

// T7c: the TauriTransport.send() entry point. When the frontend ProtocolClient sends an AgentCommand wire object as JSON it is forwarded to the daemon through the Rust DaemonClient.
// Contract: `cmd` is the externally-tagged JSON of AgentCommand (e.g. `{"Kill":{…}}`). A parse failure (JSON structure mismatch) returns an error; so does a broken envelope field of `Command` · reply field of `CommandOutcome` · registration list field of `RegisterCommands` / `UpdateCommands`.
// Reply plane (Fix-B / plan ②): the frontend ProtocolClient waits for a reply with a oneshot (pending map), receives it as `onMessage` (control InboundMessage) and resolves by request_id matching. So this invoke does not discard the reply: it serializes the daemon reply (AgentEvent) as is as the return value, TauriTransport.send raises it as `onMessage({kind:'control', event})` and ProtocolClient.handleEvent resolves the pending as before (isomorphic to WsTransport raising a daemon Text frame as control — a oneshot path with no broadcast waste or multi-window leak).
// Path branching by request_id presence (hang prevention):
// - With a request_id (spawn/kill/write/list/snapshot/profile CRUD/…) → `send_command` (awaits the reply). Every command ProtocolClient sets a pending for has one (only resizePty is an exception = fire-and-forget), and the daemon echoes a reply variant (Ack/Spawned/Created/AgentList/ProfileList/Snapshot) or Error for them (connection_core.rs dispatch) — no command leaves the `send_command` await occupied forever.
// - Without a request_id (Resize etc.) → fire-and-forget, `Ok(None)` (no reply to raise).
// - The subscription set (Subscribe/Unsubscribe/UsageSubscribe) → blocked before that, `Ok(None)` (see the body comment below).
// Return `Ok(Some(value))` = a control event (reply) to raise to the frontend, `Ok(None)` = nothing to raise.
#[tauri::command]
pub async fn forward_daemon_command(
    client: tauri::State<'_, std::sync::Arc<DaemonClient>>,
    cmd: serde_json::Value,
) -> Result<Option<serde_json::Value>, String> {
    forward_to_daemon(&client, cmd).await
}

pub(crate) async fn forward_to_daemon(
    client: &DaemonClient,
    cmd: serde_json::Value,
) -> Result<Option<serde_json::Value>, String> {
    let agent_cmd: engram_dashboard_protocol::AgentCommand =
        serde_json::from_value(cmd).map_err(|e| format!("failed to parse AgentCommand: {e}"))?;
    // The command fields (envelope · reply · registration list) are raw text, so the parse above does not look at their shape — a broken field not stopped here would send an envelope one-way with no number (the frontend pending never rejected) and let the daemon receive a reply as a numberless rejection (the frontend never learns of the failure) (ADR-0272).
    let agent_cmd = crate::daemon_client::protocol_state::reseal_webview_command_slots(agent_cmd)
        .map_err(|e| format!("failed to parse the AgentCommand command field: {e}"))?;

    // Daemon subscription ownership = src-tauri alone (ADR-0035/0037 — BLOCK-1): Subscribe/Unsubscribe the frontend sends are blocked (dropped) here, not passed to the daemon. Daemon subscribe and resubscribe are triggered only by the layout subscription delta (ViewManager authority, `send_subscription_delta` in commands/layout.rs) with `after_seq=the buffer's latest seq` (axis A) — the frontend forwarding `Subscribe{after_seq:null}` (FromOldest) from N windows would make the daemon replay FromOldest N times and collapse the monotonic shared-buffer seq (the lossless premise). The frontend ProtocolClient does not send Subscribe on the first subscribeOutput, but `resubscribeAll` (reconnect resume) or a future carrier change could, so Rust ignores it as a second line of defense (either way 0 direct daemon subscriptions).
    // The usage subscription (`UsageSubscribe`) is also shell-only (TRD S21 usage-limit-slot §1-7): the interest is computed by the shell from the layout and only the connection task uses that set; one webview page reaching the daemon would diverge the shell's `sent` set from the daemon subscription — it has no request_id, so unblocked it goes straight to the fire-and-forget below.
    if matches!(
        agent_cmd,
        engram_dashboard_protocol::AgentCommand::Subscribe { .. }
            | engram_dashboard_protocol::AgentCommand::Unsubscribe { .. }
            | engram_dashboard_protocol::AgentCommand::UsageSubscribe { .. }
    ) {
        tracing::debug!(
            cmd = ?agent_cmd,
            "forward_daemon_command: frontend subscription command blocked (output subscription = layout delta BLOCK-1 · usage subscription = shell interest)"
        );
        return Ok(None);
    }

    if crate::daemon_client::protocol_state::command_request_id(&agent_cmd).is_none() {
        client.send_fire_and_forget(agent_cmd);
        return Ok(None);
    }

    // Reply serialization: the daemon reply (Ack/Spawned/…) is an externally-tagged AgentEvent, so serializing it gives the `{"Ack":{"request_id":…}}` shape the frontend handleEvent expects. Disconnect: if `send_command` returns Err (disconnected / no response received) that message goes to the frontend so ProtocolClient.sendCommand's `send().catch` rejects the pending (no permanent hang).
    // A reply that never comes while the link stays up (the daemon drops one answer) is bounded by the transport's request timeout — the shared one of every round trip (ADR-0181 · ADR-0310 D15 ① · `daemon_client::connection::link_policy`), text `link::errors::reply_timeout_text`. No second timeout here. A command sent while the connection is live but not yet opened by the driver first waits for the open, bounded by the same value — worst case about twice it (the open gap is normally milliseconds).
    let event = client.send_command(agent_cmd).await?;
    let value = serde_json::to_value(&event).map_err(|e| format!("failed to serialize reply: {e}"))?;
    Ok(Some(value))
}

fn expect_ack(result: CmdResult) -> Result<(), String> {
    match result {
        Ok(AgentEvent::Ack { .. }) => Ok(()),
        // Some other success reply from the daemon (it should not) is treated as success — the point is that it is not an Error. Protocol drift made visible (FIX-5): swallowing a non-Ack variant silently would hide a daemon-client contract mismatch → warn which variant, then pass (the return behavior is unchanged).
        Ok(other) => {
            tracing::warn!(
                reply = ?other,
                "expect_ack: treating a reply that is not Ack as success (suspected protocol drift)"
            );
            Ok(())
        }
        Err(e) => Err(e),
    }
}

fn expect_ack_or_spawned(result: CmdResult) -> Result<(), String> {
    match result {
        Ok(AgentEvent::Ack { .. }) | Ok(AgentEvent::Spawned { .. }) => Ok(()),
        // Protocol drift made visible (FIX-5): do not silently swallow a variant other than Ack/Spawned — warn.
        Ok(other) => {
            tracing::warn!(
                reply = ?other,
                "expect_ack_or_spawned: treating a reply that is not Ack/Spawned as success (suspected protocol drift)"
            );
            Ok(())
        }
        Err(e) => Err(e),
    }
}
