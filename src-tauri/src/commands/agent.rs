//! Agent command invoke handlers — the request/reply plane (S14 module① T6a, ADR-0036).
//!
//! When the frontend (or §5 LLM) calls `invoke('agent_spawn'|…)`, this builds an `AgentCommand`, sends it to the daemon through
//! `DaemonClient::send_command`, and returns the daemon reply (matched by request_id) as a frontend-friendly Result.
//! The brain (connection semantics) is `DaemonClient`; this layer is only a **thin builder + injection**.
//!
//! ## ★request_id is set here (load-bearing)★
//! Each handler creates a new key with `RequestId::new()` and puts it in the command — send_command matches the reply by that key
//! (the "request_id source" of spike §9 G1 is this layer). By idempotency semantics *the same key on retry*
//! is the standard, but for a one-shot invoke a new key each time is enough (on a disconnect the caller re-calls = a new key). Preventing writeStdin
//! duplicates is the daemon-side dedup table's responsibility (ids.rs RequestId comment).

use std::sync::Arc;

use engram_dashboard_protocol::{
    AgentCommand, AgentEvent, AgentId, EnvelopeFormat, ProfileId, RequestId,
};
use tauri::State;
use uuid::Uuid;

use crate::daemon_client::DaemonClient;
use crate::output_channel::WindowChannelRegistry;

fn parse_uuid(s: &str, what: &str) -> Result<Uuid, String> {
    Uuid::parse_str(s).map_err(|e| format!("{what} UUID 파싱 실패: {e}"))
}

type CmdResult = Result<AgentEvent, String>;

// ★Note (reply kind)★: the daemon connection_core.rs `Spawn{profile_id}` dispatch answers with Ack (only SpawnByCwd/
// SpawnProfile answer Spawned with AgentInfo attached).
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

// Agent termination (forced closing of resources).
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

// Interrupts only the work in progress. The process survives. Terminal mode (PTY) is rejected — the terminal receives keys directly.
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

// `data` is a byte array from the frontend (key input).
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

// PTY resize. ★Note★: `Resize` has no request_id on the wire (the daemon sends no reply) so it is not a target of send_command's
// reply matching — so **fire-and-forget** (`send_fire_and_forget`) is the right answer (sending it on a
// path that expects a reply would hang forever).
//
// ★Fire-and-forget meaning★: it only enqueues and does not wait for an ack (an unapplied resize = just a screen size mismatch, harmless to
// operational safety). If not connected, DaemonClient quietly no-ops — Resize is a one-shot command, not a subscription delta, so
// it is not automatically re-synced on connect (the next resize input updates to the new dimensions). So `Ok(())` means "a send was attempted",
// not "daemon application confirmed".
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

// ★Global envelope format switch (ADR-0096) — a steering/invoke control surface★. When an operator (or §5 LLM / resident
// orchestrator) calls `invoke('set_envelope_format', { format: 'colon' | 'xml' })` it changes the daemon-global
// envelope format state — afterwards every A→B message envelope is rendered in that format (colon=`{sender}: {body}`
// default, xml=`<message from="{sender}">{body}</message>`).
//
// ★src-tauri = client shell (ADR-0029)★: state ownership and assembly belong to the daemon (the process that owns AgentManager).
// ★Not exposed on the worker MCP channel★ (a managed agent must not change a team-global format — ADR-0096 decision 3 · ADR-0094
// least privilege). This invoke surface is the official active path, and a human trigger (shortcut/UI button) is a later option
// (ADR-0096 decision 5 — this invoke is temporarily called and verified with cdp.mjs eval).
//
// ★format deserialization★: `EnvelopeFormat` is serde lowercase so the invoke JSON `{format:"xml"}` comes in
// as is (wire enum contract, ADR-0096).
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

// ★Output Channel registration (ADR-0046 — registration only, no replay trigger)★. The frontend calls it on window mount with
// `invoke('subscribe_output', { channel })` — it puts that window's output Channel into the window_label → Channel
// registry. The connection task looks at the routing table and fans out all agent output of that window to this Channel
// (the agent_id tag is built into the frame).
//
// ★Automatic window label injection★: if it takes `tauri::Window` as an argument, Tauri injects **the calling webview** →
// the label is obtained with `window.label()` (the frontend need not pass the label, and it cannot be forged). The Channel is also
// bound to the calling webview by birth (spike §7) — so the label↔Channel pair is always consistent.
//
// ★raw byte (spike §7)★: the registry type is `Channel<tauri::ipc::Response>` so the connection task
// sends raw bytes with `Response::new(bytes)` (avoiding the JSON serialization trap of `Channel<Vec<u8>>`).
//
// ## ★ADR-0046: replay trigger separated★
// With the mirror buffer removed, subscribe_output does **Channel registration only** (the old register-then-replay-immediately is deleted). Replay is triggered by the view
// separately on mount with `request_replay` — registration (per window) and replay (per view/agent) are separated so that multiple
// views of one window each know their own replay boundary through the gen fence.
#[tauri::command]
pub fn subscribe_output(
    registry: State<'_, WindowChannelRegistry>,
    window: tauri::Window,
    channel: tauri::ipc::Channel<tauri::ipc::Response>,
) -> Result<(), String> {
    let label = window.label().to_string();
    // ★ADR-0006★: the registry is a std Mutex — insert is synchronous, 0 awaits while holding the lock. Re-registering the same label (window reload)
    //   overwrites (the old Channel is dropped — harmless since that webview is already dead).
    // ★Register even if poisoned (ADR-0231)★ — the old `Err` return left that window never receiving output (the canonical
    //   reasoning = the `output_channel` module header).
    crate::output_channel::register_window(&registry, label, channel);
    Ok(())
}

// ★View-driven replay numbering (ADR-0046 M1 — request_replay)★. A view (slot) calls it on mount/remount — it triggers a full re-replay of that agent's
// daemon ring as single-flight and returns the assigned `gen` (generation). The view does sort+dedup flush only on success markers whose
// gen is **at or above** the gen it received (gen fence — blocks early flush of someone else's / an old generation's replay).
//
// ★BLOCK-1 made universal (ADR-0046)★: this is the only path that sends wire `Subscribe{after_seq:None}` (everything). layout sends
// only Unsubscribe (cleanup). If not connected, Err (the frontend re-requests on the connected transition — M2).
#[tauri::command]
pub async fn request_replay(
    client: State<'_, Arc<DaemonClient>>,
    agent_id: String,
) -> Result<u64, String> {
    let agent_id: AgentId = parse_uuid(&agent_id, "agent_id")?;
    client.request_replay(agent_id).await
}

// ★T7c: the TauriTransport.send() entry point★. When the frontend ProtocolClient sends an AgentCommand wire object as
// JSON, it is forwarded to the daemon through the Rust DaemonClient.
//
// ★Contract★: `cmd` is the externally-tagged JSON of AgentCommand (e.g. `{"Kill":{…}}`). A parse failure returns an
// error (JSON structure mismatch). A broken envelope field of `Command` · reply field of `CommandOutcome` · registration list field of `RegisterCommands` and
// `UpdateCommands` also returns the same error.
//
// ## ★reply plane (Fix-B / plan ②)★
// The frontend ProtocolClient waits for a reply with a oneshot (pending map), receives that reply as `onMessage` (control
// InboundMessage), and resolves by request_id matching. So this invoke **does not discard the reply** but
// serializes the daemon reply (AgentEvent) as is and returns it as the invoke return value — TauriTransport.send raises that return as
// `onMessage({kind:'control', event})`, and ProtocolClient.handleEvent breaks the pending as before.
// (Isomorphic to WsTransport raising a daemon Text frame as control — a oneshot path with no broadcast waste or multi-window leak.)
//
// ★Path branching by presence of request_id (hang prevention)★:
// - Commands with a request_id (spawn/kill/write/list/snapshot/profile CRUD/…) → `send_command` (awaits the reply).
//   Commands for which ProtocolClient sets a pending *all* have a request_id (only resizePty is an exception = fire-and-forget),
//   and the daemon echoes a reply variant (Ack/Spawned/Created/AgentList/ProfileList/Snapshot) or Error for those commands
//   (connection_core.rs dispatch). That is, no command leaves the send_command await occupied forever.
// - Commands without a request_id (Resize etc.) → fire-and-forget. Returns `Ok(None)` (no reply to raise).
// - The subscription set (Subscribe/Unsubscribe/UsageSubscribe) → blocked before that. Returns `Ok(None)` (see the body comment below).
//
// Return `Ok(Some(value))` = a control event (reply) to raise to the frontend, `Ok(None)` = nothing to raise.
#[tauri::command]
pub async fn forward_daemon_command(
    client: tauri::State<'_, std::sync::Arc<DaemonClient>>,
    cmd: serde_json::Value,
) -> Result<Option<serde_json::Value>, String> {
    forward_to_daemon(&client, cmd).await
}

/// The body of [`forward_daemon_command`] — the harness drives it without `State` (`daemon_client/connection/tests.rs`).
pub(crate) async fn forward_to_daemon(
    client: &DaemonClient,
    cmd: serde_json::Value,
) -> Result<Option<serde_json::Value>, String> {
    let agent_cmd: engram_dashboard_protocol::AgentCommand =
        serde_json::from_value(cmd).map_err(|e| format!("AgentCommand 파싱 실패: {e}"))?;
    // The command fields (envelope · reply · registration list) are raw text so the parsing above does not look at their shape — if a broken field is not stopped here,
    //   an envelope goes out as a one-way shot with no number so the frontend pending is never rejected, and a reply is
    //   received by the daemon as a rejection with no number so the frontend does not learn of the failure (ADR-0272).
    let agent_cmd = crate::daemon_client::protocol_state::reseal_webview_command_slots(agent_cmd)
        .map_err(|e| format!("AgentCommand 명령 칸 파싱 실패: {e}"))?;

    // ★Daemon subscription ownership = src-tauri alone (ADR-0035/0037 — BLOCK-1)★: Subscribe/Unsubscribe sent by the frontend
    //   are not passed to the daemon but blocked (dropped) here. Daemon subscribe and resubscribe are triggered by the layout subscription delta (ViewManager
    //   authority, send_subscription_delta in commands/layout.rs) alone, with `after_seq=버퍼 최신 seq` (axis A) — because if the frontend
    //   forwarded `Subscribe{after_seq:null}` (FromOldest) from N windows, the daemon would replay
    //   FromOldest N times and the monotonic shared-buffer seq (the lossless premise) would collapse. The frontend
    //   ProtocolClient already does not send Subscribe on the first subscribeOutput subscription, but resubscribeAll
    //   (reconnect resume) or a future carrier change could send it again, so Rust ignores it as a second line of defense
    //   (either the frontend does not send or Rust ignores — either way 0 direct daemon subscriptions).
    // ★The usage subscription (`UsageSubscribe`) is also shell-only (TRD S21 usage-limit-slot §1-7)★: the interest is
    //   computed by the shell from the layout and only the connection task uses that set. If one webview page reached the daemon, the set the shell knows it sent (`sent`) and
    //   the daemon subscription would diverge — it has no request_id so if not blocked it goes straight to the fire-and-forget below.
    if matches!(
        agent_cmd,
        engram_dashboard_protocol::AgentCommand::Subscribe { .. }
            | engram_dashboard_protocol::AgentCommand::Unsubscribe { .. }
            | engram_dashboard_protocol::AgentCommand::UsageSubscribe { .. }
    ) {
        tracing::debug!(
            cmd = ?agent_cmd,
            "forward_daemon_command: 프론트 구독 명령 차단(출력 구독=layout 델타 BLOCK-1 · 사용량 구독=셸 관심)"
        );
        return Ok(None);
    }

    if crate::daemon_client::protocol_state::command_request_id(&agent_cmd).is_none() {
        client.send_fire_and_forget(agent_cmd);
        return Ok(None);
    }

    // ★reply serialization★: the daemon reply (Ack/Spawned/…) is an externally-tagged AgentEvent, so serializing it gives
    //   the `{"Ack":{"request_id":…}}` shape as is — isomorphic to the wire shape the frontend handleEvent expects.
    // ★Disconnect handling★: if send_command returns Err (disconnected / no response received), that message is passed to the frontend so that
    //   the send().catch of ProtocolClient.sendCommand rejects the corresponding pending (blocks a permanent hang).
    //
    // A reply that never comes while the link stays up (the daemon drops one answer) is bounded by the transport's
    //   request timeout — the shared one of every round trip (ADR-0181 · ADR-0310 D15 ① ·
    //   `daemon_client::connection::link_policy`), whose text is `link::errors::reply_timeout_text`. No second
    //   timeout here. A command sent while the connection is live but not yet opened by the driver first waits for the
    //   open, bounded by the same value — so the worst case is about twice it (the open gap is normally milliseconds).
    let event = client.send_command(agent_cmd).await?;
    let value = serde_json::to_value(&event).map_err(|e| format!("reply 직렬화 실패: {e}"))?;
    Ok(Some(value))
}

fn expect_ack(result: CmdResult) -> Result<(), String> {
    match result {
        Ok(AgentEvent::Ack { .. }) => Ok(()),
        // Even if the daemon gives some other success reply (it should not), it is treated as success — the point is that it is not an Error.
        // ★Protocol drift made visible (FIX-5)★: if a variant other than Ack is *silently* swallowed as success, a daemon-client
        //   contract mismatch is invisible → warn which variant it was and then pass (the return behavior is unchanged = the caller does not break).
        Ok(other) => {
            tracing::warn!(
                reply = ?other,
                "expect_ack: Ack 가 아닌 reply 를 성공 처리(프로토콜 drift 의심)"
            );
            Ok(())
        }
        Err(e) => Err(e),
    }
}

fn expect_ack_or_spawned(result: CmdResult) -> Result<(), String> {
    match result {
        Ok(AgentEvent::Ack { .. }) | Ok(AgentEvent::Spawned { .. }) => Ok(()),
        // ★Protocol drift made visible (FIX-5)★: do not silently swallow a variant other than Ack/Spawned, warn.
        Ok(other) => {
            tracing::warn!(
                reply = ?other,
                "expect_ack_or_spawned: Ack/Spawned 가 아닌 reply 를 성공 처리(프로토콜 drift 의심)"
            );
            Ok(())
        }
        Err(e) => Err(e),
    }
}
