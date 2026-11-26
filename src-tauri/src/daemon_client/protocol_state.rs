//! Protocol semantics as pure state and decision functions — epoch guard · request id extraction · reply classification · command-slot edges (S14 module ①, ADR-0037).
//!
//! Moves into Rust the transport semantics of the frontend `src/api/protocolClient.ts` (ADR-0037 D1=option A): applied once before routing on the daemon's single WS connection, fanning clean chunks out to N windows. The epoch guard has its only source of truth here — there is no second JS defense, so a decision off by a hair makes output vanish from the screen. Dedup/progress is not in this layer (ADR-0046 moved it to the webview view (slot) unit).
//!
//! Pure: no socket, tokio runtime or Tauri. Functions take state (`&mut SubState`, a command, an event) and only decide; the connection driver does the effects, so unit tests run synchronously. Only the pure-decision part of the `protocolClient.test.ts` cases is ported (see the tests module doc).
//! Wire types `epoch: u32` · `seq: u64` · `request_id: RequestId(Uuid)` are the protocol crate types reused as is.

use std::fmt;

use engram_dashboard_command::CommandEnvelope;
use engram_dashboard_protocol::{AgentCommand, AgentEvent, AgentId, RequestId};
use serde_json::value::RawValue;

// ── Command field edge · request_id extraction (request/reply correlation) ──────────────────────────────
// ADR-0272 decision 4: a packet's command bus field is raw JSON, so protocol cannot read the number inside it; pairing is done by this edge that unpacks envelopes · replies. The daemon reads the number for a receiving-side refusal at its own edge.

/// Unpacks a packet's envelope field (raw JSON — `AgentCommand::Command` · `AgentEvent::CommandRequest`) into a command bus envelope.
/// `Err` = the envelope shape is wrong (ADR-0272 decision 6): treated as "no request number" — sending creates no waiting slot ([`command_request_id`] is `None`), receiving neither executes nor replies (`CommandRequest` branch of the connection loop).
pub fn envelope_from_wire(raw: &RawValue) -> Result<CommandEnvelope, serde_json::Error> {
    serde_json::from_str(raw.get())
}

/// Unpacks a packet's reply field (raw JSON — `AgentEvent::CommandReply` · `AgentCommand::CommandOutcome`) into a command bus reply (ADR-0272 decision 4).
/// `Err` = the reply shape is wrong (decision 6): also "no request number" — no waiting slot can be found, so the connection loop's `CommandReply` branch only logs and drops it ([`event_reply_request_id`] is `None`).
pub fn reply_from_wire(
    raw: &RawValue,
) -> Result<engram_dashboard_command::CommandReply, serde_json::Error> {
    serde_json::from_str(raw.get())
}

/// Wraps an inbound command's outcome into the raw field of an outcome packet (`AgentCommand::CommandOutcome`; ADR-0272 decision 4).
pub fn reply_to_wire(
    reply: &engram_dashboard_command::CommandReply,
) -> Result<Box<RawValue>, serde_json::Error> {
    serde_json::value::to_raw_value(reply)
}

/// Wraps the registration entries this shell puts up into the raw field of a registration packet (`RegisterCommands.decls` · `UpdateCommands.added`) — the whole list is one field (ADR-0272 decision 3). The guard comparing these bytes to the hand-written TS field is `command_decl_hand_written_ts_matches_rust_shape`.
pub fn decls_to_wire(
    decls: &[engram_dashboard_command::CommandDecl],
) -> Result<Box<RawValue>, serde_json::Error> {
    serde_json::value::to_raw_value(decls)
}

/// Unpacks a registration packet's entry list field into registration entries. `Err` = the list shape is wrong (ADR-0272 decision 6); the shell has no receiving path, and [`reseal_webview_command_slots`] blocks a registration the webview sent.
pub fn decls_from_wire(
    raw: &RawValue,
) -> Result<Vec<engram_dashboard_command::CommandDecl>, serde_json::Error> {
    serde_json::from_str(raw.get())
}

/// Unpacks once the command fields of a packet the webview sent (envelope · reply · registration list — `Command` · `CommandOutcome` · `RegisterCommands` · `UpdateCommands`) and wraps them again in struct-order bytes; other variants return as is.
/// The webview path unpacks from `serde_json::Value`, so raw fields hold key-sorted bytes (no `preserve_order`); re-wrapping makes the same bytes go out as when the shell sends directly. `Err` = a broken field: an envelope would flow fire-and-forget and the webview's wait hang until the reply limit, a reply would reach the daemon as a numberless refusal — so the caller fails the invoke right away. For the registration list the number is outside the field, but it fails fast in the same place.
/// ADR-0272
pub fn reseal_webview_command_slots(cmd: AgentCommand) -> Result<AgentCommand, serde_json::Error> {
    match cmd {
        AgentCommand::Command { envelope } => {
            let envelope = serde_json::value::to_raw_value(&envelope_from_wire(&envelope)?)?;
            Ok(AgentCommand::Command { envelope })
        }
        AgentCommand::CommandOutcome { reply } => {
            let reply = reply_to_wire(&reply_from_wire(&reply)?)?;
            Ok(AgentCommand::CommandOutcome { reply })
        }
        AgentCommand::RegisterCommands {
            owner,
            decls,
            catalog_version,
            request_id,
        } => Ok(AgentCommand::RegisterCommands {
            owner,
            decls: decls_to_wire(&decls_from_wire(&decls)?)?,
            catalog_version,
            request_id,
        }),
        AgentCommand::UpdateCommands {
            owner,
            added,
            removed,
            request_id,
        } => Ok(AgentCommand::UpdateCommands {
            owner,
            added: decls_to_wire(&decls_from_wire(&added)?)?,
            removed,
            request_id,
        }),
        other => Ok(other),
    }
}

/// Moves the correlation key of an envelope · reply to protocol's key, carrying the uuid as is — a new value would break the identical key across the round trip (ADR-0081 decision 4).
fn request_id_from_bus(id: engram_dashboard_command::RequestId) -> RequestId {
    RequestId(id.0)
}

/// Extracts the request_id a command carries. Side-effect commands (Spawn/Kill/…) have one; Subscribe/Unsubscribe/Resize have none (no daemon reply) → `None`. The handshake is a separate frame (`handshake::AuthFrame`), not here (ADR-0129 0-4).
/// Contract: the daemon client's pending matching (`send_command`) expects a reply, so only commands with a request_id are used; a `None` command has no key to match and would be a permanent pending, so the caller filters `None`. A `Command` with a broken envelope is also `None` (ADR-0272 decision 6); the webview's such packet is blocked by [`reseal_webview_command_slots`] first.
pub fn command_request_id(cmd: &AgentCommand) -> Option<RequestId> {
    match cmd {
        AgentCommand::Spawn { request_id, .. }
        | AgentCommand::Kill { request_id, .. }
        | AgentCommand::Interrupt { request_id, .. }
        | AgentCommand::WriteStdin { request_id, .. }
        | AgentCommand::AcquireInput { request_id, .. }
        | AgentCommand::ReleaseInput { request_id, .. }
        | AgentCommand::ListAgents { request_id }
        | AgentCommand::StopDaemon { request_id, .. }
        | AgentCommand::SpawnByCwd { request_id, .. }
        | AgentCommand::ListProfiles { request_id }
        | AgentCommand::CreateProfile { request_id, .. }
        | AgentCommand::DeleteProfile { request_id, .. }
        | AgentCommand::SpawnProfile { request_id, .. }
        | AgentCommand::SetProfileAutoRestore { request_id, .. }
        // ADR-0061
        | AgentCommand::RenameProfile { request_id, .. }
        // ADR-0072
        | AgentCommand::ReparentProfile { request_id, .. }
        | AgentCommand::GetSnapshot { request_id, .. }
        // ADR-0096
        | AgentCommand::SetEnvelopeFormat { request_id, .. }
        // Command bus registration wire (ADR-0155/0156) — all three wait for a reply (Ack for registration and diff, the dedicated `CommandList` for the query; `event_reply_request_id` below is its pair).
        | AgentCommand::RegisterCommands { request_id, .. }
        | AgentCommand::UpdateCommands { request_id, .. }
        | AgentCommand::ListCommands { request_id }
        // ADR-0231
        | AgentCommand::ListQueuedInputs { request_id, .. }
        | AgentCommand::CancelQueuedInput { request_id, .. }
        | AgentCommand::RefreshUsageLimits { request_id, .. } => Some(*request_id),
        // A command request is a correlation target — only the key sits inside the envelope (ADR-0155). `None` here would leave the shell unable to make a slot to wake, hanging to the deadline; it holds only as a pair with the `CommandReply` branch of `event_reply_request_id` below. The uuid is carried as is (`request_id_from_bus`).
        // ADR-0272 decisions 4 · 6: unpack the envelope and read the number — a broken envelope has no number.
        AgentCommand::Command { envelope } => envelope_from_wire(envelope)
            .ok()
            .map(|envelope| request_id_from_bus(envelope.request_id)),
        AgentCommand::Resize { .. }
        | AgentCommand::Subscribe { .. }
        | AgentCommand::Unsubscribe { .. }
        | AgentCommand::UsageSubscribe { .. }
        // `CommandOutcome` is a reply I send, so `None`: its correlation key belongs to a request the daemon gave me, not my pending table — `Some` would create an empty pending slot at send that stays until the connection drops.
        | AgentCommand::CommandOutcome { .. } => None,
    }
}

/// Extracts the request_id a reply event carries. Only a dedicated reply (an answer tied to one request) echoes it; broadcasts (AgentListUpdated/StatusChanged/…) are `None` and bypass pending matching. The variant list is the single match below — copying it elsewhere lets one side drift.
/// `Error{request_id: Some(_)}` = failure of a specific command (matched and rejected); `Error{request_id: None}` = unrelated to a command (not matched). SubscribeAck has no request_id (agent_id based) → `None`. A `CommandReply` with a broken reply is `None` too (ADR-0272 decision 6): that wait ends at the link's request timeout (`connection::link_policy`).
pub fn event_reply_request_id(ev: &AgentEvent) -> Option<RequestId> {
    match ev {
        AgentEvent::Ack { request_id }
        | AgentEvent::AgentList { request_id, .. }
        | AgentEvent::ProfileList { request_id, .. }
        | AgentEvent::Snapshot { request_id, .. }
        | AgentEvent::Created { request_id, .. }
        // `CommandList` is a correlation target (ADR-0155) — the dedicated reply of `ListCommands`, like AgentList/ProfileList. `None` would hang the shell: `command_request_id` returns `Some` for `ListCommands`, creating a slot with nothing to wake it until the connection drops. The two functions move together per pair; the tests below pin it.
        | AgentEvent::CommandList { request_id, .. }
        // ADR-0231
        | AgentEvent::QueuedInputs { request_id, .. }
        | AgentEvent::QueuedInputCancelReply { request_id, .. }
        | AgentEvent::Spawned { request_id, .. } => Some(*request_id),
        // A command reply is a correlation target too — the pair of the `AgentCommand::Command` branch above (ADR-0155); `Some` on the request side with `None` here leaves the slot unreleased until the connection drops.
        // ADR-0272 decisions 4 · 6: unpack the reply and read the number — a broken reply has no number (`reply_from_wire`). Picking out just the number is rejected (the ADR's 「Rejected alternatives」 — partial parsing).
        AgentEvent::CommandReply { reply } => reply_from_wire(reply)
            .ok()
            .map(|reply| request_id_from_bus(reply.request_id)),
        AgentEvent::Error { request_id, .. } => *request_id,
        AgentEvent::Hello { .. }
        | AgentEvent::SubscribeAck { .. }
        | AgentEvent::SubscribeFailed { .. }
        | AgentEvent::Output { .. }
        | AgentEvent::ReplayComplete { .. }
        | AgentEvent::StatusChanged { .. }
        | AgentEvent::AgentListUpdated { .. }
        | AgentEvent::RestoreResult { .. }
        | AgentEvent::InputLeaseChanged { .. }
        | AgentEvent::ProfileListUpdated { .. }
        | AgentEvent::UsageLimitsUpdated { .. }
        // `CommandRequest` is an incoming request, so `None` (load-bearing): its envelope's request_id is the daemon's key, not mine — `Some` makes the receiving side read it as the reply it was waiting for and swallow the envelope, so the command is not executed and the sender hangs until the deadline.
        | AgentEvent::CommandRequest { .. } => None,
    }
}

/// Who stated the failure when a command the shell sent did not end with a success answer.
/// - `Daemon` = the daemon answered that command with `Error` (with or without a code prefix, known code or not).
/// - `Local` = it ended shell-side without a daemon answer (not connected · not sent · send failed · outcome unknown by disconnect · answer lost · succession …). Does not mean "not applied": outcome-unknown branches (`link::errors::SENT_OUTCOME_UNKNOWN`) may have executed.
/// `Display` emits the held text undecorated — `DaemonClient::send_command` builds its `String` from this, so a caller ignoring the source gets the original text byte for byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandFailure {
    /// `launch_blocked` · `created_agent_id` = the `AgentEvent::Error` fields of the same names, copied as received
    /// (absent = the daemon sent none). `Display` shows `message` only.
    // ADR-0348
    Daemon {
        message: String,
        launch_blocked: Option<serde_json::Map<String, serde_json::Value>>,
        created_agent_id: Option<AgentId>,
    },
    Local(String),
}

impl fmt::Display for CommandFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandFailure::Daemon { message, .. } | CommandFailure::Local(message) => {
                f.write_str(message)
            }
        }
    }
}

/// Sorts a round trip's outcome by source (ADR-0281): `Ok(Error{..})` → `Daemon` · other `Ok` → as is · `Err` → `Local`. The caller puts the daemon's reply (its `Error` included) in `Ok` and only shell-local failures in `Err` (the driver's `send_command`) — folding an `Error` into `Err` silently kills the refusal box (ADR-0270 decision 4).
/// It does not read the text: a daemon answer without a prefix or with an unknown code is still `Daemon` (adding codes is allowed — `engram_dashboard_command::ErrorCode` doc).
pub fn classify_reply(slot: Result<AgentEvent, String>) -> Result<AgentEvent, CommandFailure> {
    match slot {
        Ok(AgentEvent::Error {
            message,
            launch_blocked,
            created_agent_id,
            ..
        }) => Err(CommandFailure::Daemon {
            message,
            launch_blocked,
            created_agent_id,
        }),
        Ok(other) => Ok(other),
        Err(local) => Err(CommandFailure::Local(local)),
    }
}

/// Per-agent output subscription state (from JS `protocolClient.ts` `SubState`). The per-agent source of truth of the epoch guard (ADR-0046) — the connection task holds an agent_id → SubState map and passes `&mut` to the decision functions on every SubscribeAck/output frame.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SubState {
    // The last `SubscribeAck.current_epoch`: matches output frames (mismatches discarded) and is the wire epoch of the reconnect resubscribe. `None` = no Ack yet.
    pub epoch: Option<u32>,
}

impl SubState {
    // Value right after a new subscription (JS `subscribeOutput`: epoch=undefined); a resubscription of the same agent_id overwrites with it.
    pub fn new() -> Self {
        Self { epoch: None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpochDecision {
    Deliver,
    DropEpochMismatch,
}

// ── output frame epoch guard ─────────────────────────────────────────────────────────
/// Decides only the epoch guard. Dedup (seq high-water) lives in the frontend view (ADR-0046).
/// Do not bring high-water back to this layer (ADR-0046 decision 3): progress held per agentId makes several views watching the same agent starve each other (view A renders seq 100, view B needing seq from 0 cannot get seq<=100 either — bug B). The epoch is per session and unrelated to views.
/// `epoch=None` (before the first Ack) passes: with no baseline the comparison is skipped so early output is not lost (as the JS `st.epoch !== undefined` guard).
pub fn decide_epoch(st: &SubState, frame_epoch: u32) -> EpochDecision {
    if let Some(cur) = st.epoch {
        if frame_epoch != cur {
            return EpochDecision::DropEpochMismatch;
        }
    }
    EpochDecision::Deliver
}

// ── SubscribeAck handling (from the SubscribeAck branch of JS handleEvent) ───────────────────────
/// Updates the epoch on SubscribeAck; returns `true` if it changed.
/// Production (`connection.rs`) discards the return with `let _ =` — since ADR-0046 src-tauri has no progress state to reset and the frontend adopts the epoch through the success marker. The return contract is pinned by this file's tests (first Ack · same epoch · change).
/// Bug B guard: `replay_from` is "the first seq the daemon sends", not "the last seq seen" — as a dedup baseline it would drop the first valid frame, so this function takes no replay_from.
/// A changed epoch (daemon · agent restart; ADR-0007, incarnation marker ADR-0163, reattach trigger ADR-0164) is a new stream: the frontend resubscribes by observing the authoritative roster (`observeRoster`), and the subscription deps `[viewId, agentId]` deliberately exclude the incarnation marker (ADR-0164 decision 8). This function only moves `SubState.epoch`.
pub fn apply_subscribe_ack(st: &mut SubState, current_epoch: u32) -> bool {
    let epoch_changed = match st.epoch {
        Some(prev) => current_epoch != prev,
        None => false, // the first setting, with no baseline to compare — None → Some is not a "change".
    };
    st.epoch = Some(current_epoch);
    epoch_changed
}

#[cfg(test)]
mod tests {
    //! Ports only the parts of the `src/api/protocolClient.test.ts` cases that are pure state/decision functions (each test names the matching TS case); not 1:1 — a TS case mixing routing and decision ports only the decision.
    //!
    //! No seq dedup/high-water tests: that state is not in src-tauri (ADR-0046). Pinned: slot unwrapping (envelope · reply · declaration), request id extraction, reply classification, the epoch guard, `apply_subscribe_ack`'s return value.
    //! Event routing is not verified here: variant routing + callback calls/unsubscribe are not pure decisions and protocol_state alone has nothing to protect — they are covered by the connection driver's routing (`link::route`) and the eventBus surface tests (see the event-routing comment below). transport.close/connect/disconnect delegation (`:399~`, `:411~`) is carrier wiring and not here.

    use super::*;
    use engram_dashboard_protocol::AgentCommand;

    // ── Event-routing cases (not verified here — on purpose) ──────────────────────────
    // These TS cases are variant routing plus callback calls / unsubscribes; a vacuous re-check here would only give false confidence. They are verified in `link::route` and the eventBus surface tests:
    //   - broadcast-no-consume (`protocolClient.test.ts:141`): a broadcast (AgentListUpdated) wakes no waiting request.
    //   - two-concurrent-ordering (`:161`): replies resolve in any arrival order — reply routing by id.
    //   - StatusChanged callback + off()/unsubscribe (`:302`) · RestoreResult{report} callback (`:315`) · ProfileListUpdated callback + off()/unsubscribe (`:325`)

    // ── epoch guard (decide_epoch) ───────────────────────────────────────────────────

    fn subscribed_with_ack(epoch: u32) -> SubState {
        let mut st = SubState::new();
        apply_subscribe_ack(&mut st, epoch);
        st
    }

    #[test]
    fn epoch_mismatch_dropped() {
        let st = subscribed_with_ack(5);
        assert_eq!(decide_epoch(&st, 4), EpochDecision::DropEpochMismatch);
        assert_eq!(decide_epoch(&st, 5), EpochDecision::Deliver);
    }

    #[test]
    fn frame_before_ack_passes_epoch_guard() {
        let st = SubState::new(); // before the Ack — epoch=None
        assert_eq!(st.epoch, None);
        assert_eq!(decide_epoch(&st, 99), EpochDecision::Deliver);
        assert_eq!(decide_epoch(&st, 0), EpochDecision::Deliver);
    }

    #[test]
    fn ack_epoch1_output_epoch1_delivered() {
        let st = subscribed_with_ack(1);
        assert_eq!(decide_epoch(&st, 1), EpochDecision::Deliver);
    }

    #[test]
    fn ack_epoch1_output_epoch0_dropped() {
        let st = subscribed_with_ack(1);
        assert_eq!(decide_epoch(&st, 0), EpochDecision::DropEpochMismatch);
    }

    #[test]
    fn fresh_session_epoch0_delivered() {
        let st = subscribed_with_ack(0);
        assert_eq!(decide_epoch(&st, 0), EpochDecision::Deliver);
    }

    // ── apply_subscribe_ack — bool return ───────────────────────────────────────────

    #[test]
    fn first_ack_returns_false() {
        let mut st = SubState::new();
        let changed = apply_subscribe_ack(&mut st, 10);
        assert!(
            !changed,
            "the first Ack is no epoch change (None → Some, but no reset needed)"
        );
        assert_eq!(st.epoch, Some(10));
    }

    #[test]
    fn same_epoch_ack_returns_false() {
        let mut st = subscribed_with_ack(5);
        let changed = apply_subscribe_ack(&mut st, 5);
        assert!(!changed, "same epoch reconfirmed — no window reset needed");
    }

    /// Epoch change Ack — changed=true (pins the return contract: a value different from the previous epoch counts as a change).
    #[test]
    fn epoch_change_ack_returns_true() {
        let mut st = subscribed_with_ack(10);
        let changed = apply_subscribe_ack(&mut st, 11);
        assert!(
            changed,
            "epoch 11→10 change — window render_seq reset needed"
        );
        assert_eq!(st.epoch, Some(11));
    }

    // ── Why there are no resubscribe_params/initial_subscribe_params tests (ADR-0046) ────────
    // src-tauri does not reissue Subscribe on reconnect nor compute initial subscription parameters — the wire subscription comes from the frontend request_replay alone (view-driven full re-replay), so there is no function to pin.

    // ── request_id extraction + reply outcome classification ──────────────────────────────────────────

    // Side-effect commands return a request_id; Auth/Resize/Subscribe/Unsubscribe are `None` — a unit pin of the contract where `send_command` filters `None` to prevent a permanent pending.
    #[test]
    fn command_request_id_extracts_or_none() {
        let r = RequestId::new();
        let spawn = AgentCommand::Spawn {
            profile_id: uuid::Uuid::new_v4(),
            request_id: r,
        };
        assert_eq!(
            command_request_id(&spawn),
            Some(r),
            "Spawn carries a request_id"
        );

        let kill = AgentCommand::Kill {
            agent_id: uuid::Uuid::new_v4(),
            request_id: r,
        };
        assert_eq!(command_request_id(&kill), Some(r));

        let resize = AgentCommand::Resize {
            agent_id: uuid::Uuid::new_v4(),
            cols: 80,
            rows: 24,
            viewport_id: None,
        };
        assert_eq!(
            command_request_id(&resize),
            None,
            "Resize has no request_id"
        );
        let sub = AgentCommand::Subscribe {
            agent_id: uuid::Uuid::new_v4(),
            epoch: None,
            after_seq: None,
        };
        assert_eq!(
            command_request_id(&sub),
            None,
            "Subscribe has no request_id"
        );
        // ADR-0129
    }

    #[test]
    fn event_reply_request_id_only_for_replies() {
        let r = RequestId::new();
        assert_eq!(
            event_reply_request_id(&AgentEvent::Ack { request_id: r }),
            Some(r),
            "Ack is a reply"
        );
        assert_eq!(
            event_reply_request_id(&AgentEvent::Error {
                request_id: Some(r),
                message: "x".into(),
                launch_blocked: None,
                created_agent_id: None,
            }),
            Some(r),
            "Error{{Some}} is a specific command failure — matched"
        );
        assert_eq!(
            event_reply_request_id(&AgentEvent::Error {
                request_id: None,
                message: "x".into(),
                launch_blocked: None,
                created_agent_id: None,
            }),
            None,
            "Error{{None}} is unrelated to a command — not matched"
        );
        assert_eq!(
            event_reply_request_id(&AgentEvent::AgentListUpdated { agents: vec![] }),
            None,
            "AgentListUpdated is a broadcast — bypasses matching"
        );
        // The ADR-0155 pair (ListCommands↔CommandList) is pinned by `list_commands_reply_request_id_pairing` below; SubscribeAck is agent_id based (no request_id).
        assert_eq!(
            event_reply_request_id(&AgentEvent::SubscribeAck {
                agent_id: uuid::Uuid::new_v4(),
                action: engram_dashboard_protocol::SubscribeAction::Reset,
                current_epoch: 0,
                oldest_seq: 0,
                latest_seq: 0,
                replay_from: 0,
                truncated: false,
                continues_conversation: false,
            }),
            None
        );
    }

    // classify_reply: a daemon-answered `Error` is Daemon regardless of text · a slot `Err` is Local · else Ok. Unknown codes and text without a prefix are this test's job — a text-splitting classifier would pass with just the two known codes.
    #[test]
    fn classify_reply_keeps_origin_without_reading_text() {
        let r = RequestId::new();
        for message in [
            "CONFLICT: …",
            "INVALID_ARGUMENT: …",
            "BRAND_NEW_CODE: …",
            "text without a prefix",
        ] {
            let classified = classify_reply(Ok(AgentEvent::Error {
                request_id: Some(r),
                message: message.into(),
                launch_blocked: None,
                created_agent_id: None,
            }));
            match classified {
                Err(failure @ CommandFailure::Daemon { .. }) => {
                    assert_eq!(
                        failure.to_string(),
                        message,
                        "Display = the daemon's original text"
                    )
                }
                other => panic!("a daemon Error must be Daemon — {message:?}: {other:?}"),
            }
        }

        let local = crate::daemon_client::link::errors::SENT_OUTCOME_UNKNOWN;
        match classify_reply(Err(local.to_string())) {
            Err(failure @ CommandFailure::Local(_)) => {
                assert_eq!(failure.to_string(), local, "Display = the local text as is")
            }
            other => panic!("a slot Err must be Local: {other:?}"),
        }

        match classify_reply(Ok(AgentEvent::Ack { request_id: r })) {
            Ok(AgentEvent::Ack { request_id }) => assert_eq!(request_id, r),
            other => panic!("Ack must stay Ok: {other:?}"),
        }
    }

    /// The structured fields of a daemon `Error` survive the sort — the shell bus `agent.spawnInto` answers with them
    /// (`args.reasons` · `args.agent_id`) and has nothing else to read them from.
    // ADR-0348
    #[test]
    fn classify_reply_keeps_the_launch_block_and_the_created_agent() {
        let blocked = serde_json::json!({ "reasons": [{ "kind": "missing", "item": "C:/gone" }] });
        let blocked = blocked.as_object().cloned();
        let created = uuid::Uuid::new_v4();
        let classified = classify_reply(Ok(AgentEvent::Error {
            request_id: Some(RequestId::new()),
            message: "INTERNAL: agent 'a' was created but did not start".into(),
            launch_blocked: blocked.clone(),
            created_agent_id: Some(created),
        }));
        assert_eq!(
            classified.expect_err("a daemon Error"),
            CommandFailure::Daemon {
                message: "INTERNAL: agent 'a' was created but did not start".into(),
                launch_blocked: blocked,
                created_agent_id: Some(created),
            }
        );
    }

    // ── Command field edge · command↔reply pair (ADR-0272 decision 4) ──────────────────────────────────

    /// The same bytes as the protocol envelope golden (`NIL_ENVELOPE` in `messages.rs`) — the packet golden only measures that the bytes cross the raw field; envelope struct ↔ bytes is measured by this edge.
    const NIL_ENVELOPE: &str = r#"{"name":"tab.create","request_id":"00000000-0000-0000-0000-000000000000","owner":"shell","proto_ver":1,"args":{"window":"main"}}"#;

    fn nil_bus_envelope() -> CommandEnvelope {
        CommandEnvelope {
            name: "tab.create".to_string(),
            request_id: engram_dashboard_command::RequestId(uuid::Uuid::nil()),
            owner: engram_dashboard_command::OwnerToken::new("shell"),
            proto_ver: 1,
            args: serde_json::json!({ "window": "main" }),
        }
    }

    /// The same bytes as the protocol reply goldens (`NIL_FAILED_REPLY` · `NIL_OK_REPLY` in `messages.rs`); reply struct ↔ bytes is measured by this edge.
    const NIL_FAILED_REPLY: &str = r#"{"request_id":"00000000-0000-0000-0000-000000000000","outcome":{"Err":{"code":"NOT_FOUND","message":"no view 'x'","retry":"never"}}}"#;
    const NIL_OK_REPLY: &str = r#"{"request_id":"00000000-0000-0000-0000-000000000000","outcome":{"Ok":{"view_id":"v1"}}}"#;

    fn nil_bus_replies() -> [(engram_dashboard_command::CommandReply, &'static str); 2] {
        let nil = engram_dashboard_command::RequestId(uuid::Uuid::nil());
        [
            (
                engram_dashboard_command::CommandReply::err(
                    nil,
                    engram_dashboard_command::CommandError::of(
                        engram_dashboard_command::ErrorCode::NotFound,
                        "no view 'x'",
                    ),
                ),
                NIL_FAILED_REPLY,
            ),
            (
                engram_dashboard_command::CommandReply::ok(
                    nil,
                    serde_json::json!({ "view_id": "v1" }),
                ),
                NIL_OK_REPLY,
            ),
        ]
    }

    fn raw(json: &str) -> Box<RawValue> {
        RawValue::from_string(json.to_string()).expect("valid JSON")
    }

    /// Golden against the hand-written TS envelope field (`#[ts(type = …)]` of `AgentCommand::Command` · `AgentEvent::CommandRequest`): protocol cannot see the raw field's shape, so this pins that the envelope struct goes out as those bytes and those bytes unpack into the same struct.
    #[test]
    fn the_envelope_slot_unpacks_to_the_bus_envelope_byte_for_byte() {
        let sealed = serde_json::value::to_raw_value(&nil_bus_envelope()).unwrap();
        assert_eq!(
            sealed.get(),
            NIL_ENVELOPE,
            "envelope struct → raw field bytes differ from the golden — fix the hand-written TS field too"
        );
        assert_eq!(
            envelope_from_wire(&raw(NIL_ENVELOPE)).unwrap(),
            nil_bus_envelope()
        );
    }

    /// A packet that unpacked with a broken envelope = no request number (ADR-0272 decision 6) — a waiting slot would hang until the reply limit, since no answer comes with that number.
    #[test]
    fn a_broken_envelope_has_no_request_id() {
        for broken in [
            r#"{"name":"tab.create"}"#,
            r#"{"name":"x","request_id":"not-a-uuid","owner":"o","proto_ver":1,"args":null}"#,
            "[]",
            "null",
        ] {
            assert!(envelope_from_wire(&raw(broken)).is_err(), "{broken}");
            assert_eq!(
                command_request_id(&AgentCommand::Command {
                    envelope: raw(broken)
                }),
                None,
                "{broken}"
            );
        }
    }

    /// The webview path (`forward_daemon_command`) unpacks from `serde_json::Value`; the raw field must also unpack along it and its number be read to take the reply-waiting branch (`Value` deserialization re-serializes the raw field).
    #[test]
    fn a_command_parsed_from_a_json_value_still_pairs() {
        let sent: AgentCommand = serde_json::from_value(serde_json::json!({
            "Command": { "envelope": serde_json::from_str::<serde_json::Value>(NIL_ENVELOPE).unwrap() }
        }))
        .expect("the raw field unpacks from a Value too");
        assert_eq!(
            command_request_id(&sent),
            Some(RequestId(uuid::Uuid::nil()))
        );
    }

    /// The envelope resealed on the webview path = the struct-order golden bytes — `Value` key-sorts even a shuffled envelope, so without resealing the bytes would differ from a direct send.
    #[test]
    fn a_webview_envelope_is_resealed_into_the_golden_bytes() {
        let shuffled = serde_json::json!({
            "Command": { "envelope": {
                "args": { "window": "main" },
                "proto_ver": 1,
                "owner": "shell",
                "request_id": "00000000-0000-0000-0000-000000000000",
                "name": "tab.create",
            } }
        });
        let sent: AgentCommand = serde_json::from_value(shuffled).unwrap();
        match reseal_webview_command_slots(sent).expect("a sound envelope gets resealed") {
            AgentCommand::Command { envelope } => assert_eq!(envelope.get(), NIL_ENVELOPE),
            other => panic!("must stay Command: {other:?}"),
        }
    }

    /// A broken envelope on the webview path = reseal failure: `forward_to_daemon` turns it into an invoke failure and rejects the frontend pending right away. Commands without a command field are untouched.
    #[test]
    fn a_broken_webview_envelope_fails_to_reseal() {
        let sent: AgentCommand = serde_json::from_value(serde_json::json!({
            "Command": { "envelope": { "name": "tab.create" } }
        }))
        .expect("the raw field passes packet parsing");
        assert!(reseal_webview_command_slots(sent).is_err());

        let resize = AgentCommand::Resize {
            agent_id: uuid::Uuid::nil(),
            cols: 80,
            rows: 24,
            viewport_id: None,
        };
        assert!(matches!(
            reseal_webview_command_slots(resize),
            Ok(AgentCommand::Resize { cols: 80, .. })
        ));
    }

    /// Golden against the hand-written TS reply field (`#[ts(type = …)]` of `AgentCommand::CommandOutcome` · `AgentEvent::CommandReply`): pins that the reply struct goes out as those bytes (sent by [`reply_to_wire`]) and those bytes unpack into the same struct.
    #[test]
    fn the_reply_slot_unpacks_to_the_bus_reply_byte_for_byte() {
        for (reply, golden) in nil_bus_replies() {
            assert_eq!(
                reply_to_wire(&reply).unwrap().get(),
                golden,
                "reply struct → raw field bytes differ from the golden — fix the hand-written TS field too"
            );
            assert_eq!(reply_from_wire(&raw(golden)).unwrap(), reply);
        }
    }

    /// A packet that unpacked with a broken reply = no request number (ADR-0272 decision 6): no slot to wake, so the connection loop drops it. A fitting number in the raw text is not picked out (the ADR's 「Rejected alternatives」 — partial parsing).
    #[test]
    fn a_broken_reply_has_no_request_id() {
        for broken in [
            r#"{"request_id":"00000000-0000-0000-0000-000000000000"}"#,
            r#"{"request_id":"not-a-uuid","outcome":{"Ok":null}}"#,
            "[]",
            "null",
        ] {
            assert!(reply_from_wire(&raw(broken)).is_err(), "{broken}");
            assert_eq!(
                event_reply_request_id(&AgentEvent::CommandReply { reply: raw(broken) }),
                None,
                "{broken}"
            );
        }
    }

    /// A reply with the leniency the TS field advertises (three error fields absent · `null` · off-contract fields) also finds its pair — reading it as broken would hang the wait until the reply limit though the peer answered (the leniency is measured by the daemon's `the_outcome_slot_accepts_everything_its_ts_type_advertises`).
    #[test]
    fn a_sparse_reply_still_pairs() {
        let nil = "00000000-0000-0000-0000-000000000000";
        for sparse in [
            format!(r#"{{"request_id":"{nil}","outcome":{{"Err":{{"detail":{{"n":1}}}}}}}}"#),
            format!(
                r#"{{"request_id":"{nil}","outcome":{{"Err":{{"code":null,"message":null,"retry":null}}}}}}"#
            ),
        ] {
            assert_eq!(
                event_reply_request_id(&AgentEvent::CommandReply {
                    reply: raw(&sparse)
                }),
                Some(RequestId(uuid::Uuid::nil())),
                "{sparse}"
            );
        }
    }

    /// A woken reply goes up to the webview as a JSON value (`serde_json::to_value` in `forward_to_daemon`); the raw field must also become a value along that path so the frontend's shape is unchanged.
    #[test]
    fn a_reply_event_becomes_the_same_json_value_for_the_webview() {
        let answered = AgentEvent::CommandReply {
            reply: raw(NIL_OK_REPLY),
        };
        assert_eq!(
            serde_json::to_value(&answered).unwrap(),
            serde_json::json!({ "CommandReply": { "reply": {
                "request_id": "00000000-0000-0000-0000-000000000000",
                "outcome": { "Ok": { "view_id": "v1" } },
            } } })
        );
    }

    #[test]
    fn a_webview_outcome_is_resealed_into_the_golden_bytes() {
        let shuffled = serde_json::json!({
            "CommandOutcome": { "reply": {
                "outcome": { "Ok": { "view_id": "v1" } },
                "request_id": "00000000-0000-0000-0000-000000000000",
            } }
        });
        let sent: AgentCommand = serde_json::from_value(shuffled).unwrap();
        match reseal_webview_command_slots(sent).expect("a sound reply gets resealed") {
            AgentCommand::CommandOutcome { reply } => assert_eq!(reply.get(), NIL_OK_REPLY),
            other => panic!("must stay CommandOutcome: {other:?}"),
        }
    }

    /// Off-contract fields of a webview reply survive resealing (three fields absent + off-contract `detail` is accepted and `detail` reaches the daemon as is); daemon-side pair: `the_outcome_slot_accepts_everything_its_ts_type_advertises` in `connection_core`.
    #[test]
    fn a_webview_outcome_keeps_its_out_of_contract_fields_when_resealed() {
        let sent: AgentCommand = serde_json::from_value(serde_json::json!({
            "CommandOutcome": { "reply": {
                "request_id": "00000000-0000-0000-0000-000000000000",
                "outcome": { "Err": { "detail": { "n": 1 } } },
            } }
        }))
        .unwrap();
        match reseal_webview_command_slots(sent)
            .expect("a failure reply without the three fields gets resealed too")
        {
            AgentCommand::CommandOutcome { reply } => assert!(
                reply.get().contains(r#""detail":{"n":1}"#),
                "an off-contract field vanished in resealing: {}",
                reply.get()
            ),
            other => panic!("must stay CommandOutcome: {other:?}"),
        }
    }

    /// A broken reply on the webview path = reseal failure — let through, the daemon would receive a numberless refusal and the webview never learn of the failure; `forward_to_daemon` turns it into an invoke failure.
    #[test]
    fn a_broken_webview_reply_fails_to_reseal() {
        let sent: AgentCommand = serde_json::from_value(serde_json::json!({
            "CommandOutcome": { "reply": { "request_id": "00000000-0000-0000-0000-000000000000" } }
        }))
        .expect("the raw field passes packet parsing");
        assert!(reseal_webview_command_slots(sent).is_err());
    }

    // ── Registration entry list field (ADR-0272 decision 3) ─────────────────────────────────────────
    //
    // The entry → bytes golden and the hand-written TS field guard live here because this edge wraps the field (TRD trd-3 §6-1 U4 · U5): protocol does not know the field's inside and the daemon emits no TS (no `ts-rs`). The literals equal the protocol registration golden (`MIXED_DECLS` in `messages.rs`) · the daemon `connection_core` tests.

    const MIXED_DECLS: &str = r#"[{"name":"slot.focus","help":"{}","agent_args":["agent","after_tab"]},{"name":"tab.focus","help":"{}"}]"#;

    fn mixed_decls() -> Vec<engram_dashboard_command::CommandDecl> {
        vec![
            engram_dashboard_command::CommandDecl {
                name: "slot.focus".into(),
                help: "{}".into(),
                agent_args: vec!["agent".into(), "after_tab".into()],
                deadline: engram_dashboard_command::DeadlineClass::Ordinary,
            },
            engram_dashboard_command::CommandDecl {
                name: "tab.focus".into(),
                help: "{}".into(),
                agent_args: Vec::new(),
                deadline: engram_dashboard_command::DeadlineClass::Ordinary,
            },
        ]
    }

    /// Binds the TypeScript hand-written with `#[ts(type = "…")]` (protocol `RegisterCommands.decls` · `UpdateCommands.added`) to the Rust shape of the tool crate's `CommandDecl`.
    /// This string is the only drift risk on this path: `CommandDecl` is owned by the tool crate and does not implement `TS` (to keep that crate's dependencies minimal), so a person writes the binding text — a field added or renamed compiles in Rust while only the generated TS lies, hence the comparison.
    #[test]
    fn command_decl_hand_written_ts_matches_rust_shape() {
        use engram_dashboard_command::CommandDecl;
        use std::collections::{BTreeMap, BTreeSet};
        use ts_rs::TS;

        const DECLS_TS: &str = r#"Array<{ name: string, help: string, agent_args?: Array<string>, deadline?: "Ordinary" | "Launch" }>"#;

        let inlined = <AgentCommand as TS>::inline();
        assert!(
            inlined.matches(DECLS_TS).count() == 2,
            "the generated TS differs from the hand-written text (2 registration places) — if you fixed the attribute, fix this too:\n{inlined}"
        );

        // Fill every field so even a field dropped from serialization when empty (`agent_args`) gets its type compared.
        let decl = CommandDecl {
            name: "agent.spawn".into(),
            help: r#"{"name":"agent.spawn"}"#.into(),
            agent_args: vec!["target".into()],
            deadline: engram_dashboard_command::DeadlineClass::Launch,
        };
        let ts_type_of = |key: &str, value: &serde_json::Value| -> &'static str {
            match value {
                // The deadline class is a closed word set — the TS text spells its words.
                serde_json::Value::String(_) if key == "deadline" => r#""Ordinary" | "Launch""#,
                serde_json::Value::String(_) => "string",
                serde_json::Value::Array(items) if items.iter().all(|item| item.is_string()) => {
                    "Array<string>"
                }
                other => panic!("{key} = {other} — the TS text above does not describe this shape"),
            }
        };
        let object_of = |decl: &CommandDecl| {
            serde_json::to_value(decl)
                .expect("serialize CommandDecl")
                .as_object()
                .expect("CommandDecl goes out as a JSON object")
                .clone()
        };
        let rust_fields: BTreeMap<String, &'static str> = object_of(&decl)
            .iter()
            .map(|(k, v)| (k.clone(), ts_type_of(k, v)))
            .collect();
        // Fields dropped from serialization when empty must be optional (`?`) on the TS side.
        let sparse = object_of(&CommandDecl {
            agent_args: Vec::new(),
            deadline: engram_dashboard_command::DeadlineClass::Ordinary,
            ..decl.clone()
        });
        let skipped_when_empty: BTreeSet<String> = rust_fields
            .keys()
            .filter(|k| !sparse.contains_key(*k))
            .cloned()
            .collect();

        // A hole `rust_fields` alone does not catch: a `#[serde(skip_serializing_if = "Option::is_none")]` field vanishes from the serialized result when `None`, so a new optional field `None` in this fixture is absent from both rust_fields and (TS forgotten) ts_fields, and the first assert passes silently. Debug prints every field regardless of serde attributes, so the real field set is extracted from it (top-level field lines of `{:#?}` only — 4-space indent; deeper indent is a nested value) and compared with ts_fields.
        let debug_fields: BTreeSet<String> = format!("{decl:#?}")
            .lines()
            .filter_map(|line| {
                let rest = line.strip_prefix("    ")?;
                if rest.starts_with(' ') {
                    return None;
                }
                let (name, _) = rest.split_once(':')?;
                Some(name.trim().to_string())
            })
            .collect();

        let mut ts_optional: BTreeSet<String> = BTreeSet::new();
        let ts_fields: BTreeMap<String, &'static str> = DECLS_TS
            .trim_start_matches("Array<{")
            .trim_end_matches("}>")
            .split(',')
            .filter(|s| !s.trim().is_empty())
            .map(|pair| {
                let (name, ty) = pair.split_once(':').expect("`name: type` shape");
                let name = name.trim();
                let name = match name.strip_suffix('?') {
                    Some(optional) => {
                        ts_optional.insert(optional.to_string());
                        optional
                    }
                    None => name,
                };
                (name.to_string(), ty.trim())
            })
            .collect();

        assert_eq!(
            rust_fields, ts_fields,
            "the Rust fields · types of CommandDecl and the hand-written TS fields · types diverged"
        );
        assert_eq!(
            skipped_when_empty, ts_optional,
            "fields dropped from serialization when empty and the TS `?` fields diverged — writing a dropped field as required makes the TS lie"
        );
        let ts_names: BTreeSet<String> = ts_fields.keys().cloned().collect();
        assert_eq!(
            debug_fields, ts_names,
            "the actual fields of CommandDecl (by Debug, not hidden even by skip_serializing_if) and the hand-written \
             TS fields diverged — the rust_fields comparison above cannot catch a field dropped from serialization because its value is None"
        );
    }

    /// Pins the bytes of one registration entry (with and without `agent_args`, the optional field of ADR-0308) and of the list field wrapping them — equal to the protocol registration packet golden's field.
    #[test]
    fn command_decl_json_golden_with_and_without_agent_args() {
        let decls = mixed_decls();
        for (decl, golden) in decls.iter().zip([
            r#"{"name":"slot.focus","help":"{}","agent_args":["agent","after_tab"]}"#,
            r#"{"name":"tab.focus","help":"{}"}"#,
        ]) {
            assert_eq!(
                serde_json::to_string(decl).unwrap(),
                golden,
                "CommandDecl wire shape differs from the golden — fix the hand-written TS field too"
            );
            let back: engram_dashboard_command::CommandDecl = serde_json::from_str(golden).unwrap();
            assert_eq!(&back, decl);
        }
        assert_eq!(decls_to_wire(&decls).unwrap().get(), MIXED_DECLS);
        assert_eq!(decls_from_wire(&raw(MIXED_DECLS)).unwrap(), decls);
    }

    /// The registration list resealed on the webview path = the struct-order golden bytes (same reason as envelope · reply); what is outside the field (`owner` · `removed` · number) crosses as is.
    #[test]
    fn webview_registrations_are_resealed_into_the_golden_bytes() {
        let shuffled = serde_json::json!([
            { "help": "{}", "agent_args": ["agent", "after_tab"], "name": "slot.focus" },
            { "name": "tab.focus", "help": "{}" },
        ]);
        let registered: AgentCommand = serde_json::from_value(serde_json::json!({
            "RegisterCommands": {
                "owner": "shell",
                "decls": shuffled,
                "catalog_version": 3,
                "request_id": "00000000-0000-0000-0000-000000000000",
            }
        }))
        .unwrap();
        match reseal_webview_command_slots(registered).expect("a sound list gets resealed") {
            AgentCommand::RegisterCommands {
                owner,
                decls,
                catalog_version,
                request_id,
            } => {
                assert_eq!(decls.get(), MIXED_DECLS);
                assert_eq!(owner.as_str(), "shell");
                assert_eq!(catalog_version, 3);
                assert_eq!(request_id, RequestId(uuid::Uuid::nil()));
            }
            other => panic!("must stay RegisterCommands: {other:?}"),
        }

        let updated: AgentCommand = serde_json::from_value(serde_json::json!({
            "UpdateCommands": {
                "owner": "shell",
                "added": shuffled,
                "removed": ["chat.reset"],
                "request_id": "00000000-0000-0000-0000-000000000000",
            }
        }))
        .unwrap();
        match reseal_webview_command_slots(updated).expect("a sound list gets resealed") {
            AgentCommand::UpdateCommands { added, removed, .. } => {
                assert_eq!(added.get(), MIXED_DECLS);
                assert_eq!(removed, vec!["chat.reset".to_string()]);
            }
            other => panic!("must stay UpdateCommands: {other:?}"),
        }
    }

    #[test]
    fn a_broken_webview_registration_fails_to_reseal() {
        for broken in [
            serde_json::json!([{ "name": 1 }]),
            serde_json::json!({ "name": "tab.focus", "help": "{}" }),
        ] {
            let registered: AgentCommand = serde_json::from_value(serde_json::json!({
                "RegisterCommands": {
                    "owner": "shell",
                    "decls": broken.clone(),
                    "catalog_version": 1,
                    "request_id": "00000000-0000-0000-0000-000000000000",
                }
            }))
            .expect("the raw field passes packet parsing");
            assert!(
                reseal_webview_command_slots(registered).is_err(),
                "{broken}"
            );

            let updated: AgentCommand = serde_json::from_value(serde_json::json!({
                "UpdateCommands": {
                    "owner": "shell",
                    "added": broken.clone(),
                    "removed": [],
                    "request_id": "00000000-0000-0000-0000-000000000000",
                }
            }))
            .expect("the raw field passes packet parsing");
            assert!(reseal_webview_command_slots(updated).is_err(), "{broken}");
        }
    }

    /// The two relay-leg variants are outside pending matching: one is an incoming request and the other an outgoing reply — `Some` for either swallows the envelope or leaves an empty slot forever.
    #[test]
    fn the_relay_leg_is_outside_pending_correlation() {
        assert_eq!(
            event_reply_request_id(&AgentEvent::CommandRequest {
                envelope: raw(NIL_ENVELOPE),
            }),
            None,
            "reading an incoming request as \"the reply I was waiting for\" makes that command vanish without executing"
        );
        assert_eq!(
            command_request_id(&AgentCommand::CommandOutcome {
                reply: raw(NIL_OK_REPLY),
            }),
            None,
            "creating a pending slot while sending a reply leaves no pair to wake it"
        );
    }

    /// The two sender-leg variants are inside pending matching as a pair — the opposite of the relay leg. With only one side `Some` the round trip never closes (request only: nothing wakes the slot; reply only: no slot to wake) and the shell hangs until the deadline or disconnect. Catches an edit that fixed one side.
    #[test]
    fn the_sender_leg_is_inside_pending_correlation_as_a_pair() {
        let envelope = CommandEnvelope {
            request_id: engram_dashboard_command::RequestId::new(),
            ..nil_bus_envelope()
        };
        let sent = AgentCommand::Command {
            envelope: serde_json::value::to_raw_value(&envelope).unwrap(),
        };
        let answered = AgentEvent::CommandReply {
            reply: reply_to_wire(&engram_dashboard_command::CommandReply::ok(
                envelope.request_id,
                serde_json::json!({ "view_id": "v1" }),
            ))
            .unwrap(),
        };

        let asked =
            command_request_id(&sent).expect("if the request cannot create a pending slot, the reply cannot wake it even when it comes");
        let woke = event_reply_request_id(&answered)
            .expect("if the reply is outside correlation, there is no pair to wake that slot");
        assert_eq!(
            asked, woke,
            "both legs must see the same key for the round trip to close"
        );
        assert_eq!(
            asked.0, envelope.request_id.0,
            "must be the uuid the envelope carries as is"
        );
    }

    /// Pins the command↔reply pair (ADR-0155): `ListCommands` creates the pending slot and `CommandList` wakes it; fixing one side (e.g. a new reply variant misclassified as a broadcast) leaves the round trip unreleased until disconnect.
    #[test]
    fn list_commands_reply_request_id_pairing() {
        let r = RequestId::new();
        let cmd = AgentCommand::ListCommands { request_id: r };
        assert_eq!(
            command_request_id(&cmd),
            Some(r),
            "ListCommands carries a request_id — the side that creates the pending slot"
        );
        let reply = AgentEvent::CommandList {
            request_id: r,
            entries: vec![],
        };
        assert_eq!(
            event_reply_request_id(&reply),
            Some(r),
            "CommandList is the dedicated reply of ListCommands — it must match with the same request_id for the slot to wake"
        );
    }

    #[test]
    fn queued_input_commands_and_replies_pair_by_request_id() {
        let r = RequestId::new();
        let agent_id = uuid::Uuid::new_v4();
        let pairs = [
            (
                AgentCommand::ListQueuedInputs {
                    agent_id,
                    request_id: r,
                },
                AgentEvent::QueuedInputs {
                    request_id: r,
                    agent_id,
                    inputs: vec![],
                    as_of_seq: None,
                    epoch: 0,
                    stopped_after_error: false,
                },
            ),
            (
                AgentCommand::CancelQueuedInput {
                    agent_id,
                    input_id: "q1".into(),
                    request_id: r,
                },
                AgentEvent::QueuedInputCancelReply {
                    request_id: r,
                    agent_id,
                    input_id: "q1".into(),
                    outcome: "cancelled".into(),
                },
            ),
        ];
        for (cmd, reply) in pairs {
            assert_eq!(command_request_id(&cmd), Some(r), "{cmd:?}");
            assert_eq!(event_reply_request_id(&reply), Some(r), "{reply:?}");
        }
    }

    #[test]
    fn usage_wire_request_id_arms() {
        use engram_dashboard_protocol::AgentBackendKind;

        let r = RequestId::new();
        assert_eq!(
            command_request_id(&AgentCommand::UsageSubscribe {
                vendors: vec![AgentBackendKind::Claude],
            }),
            None
        );
        assert_eq!(
            command_request_id(&AgentCommand::RefreshUsageLimits {
                vendor: AgentBackendKind::Claude,
                request_id: r,
            }),
            Some(r)
        );
        assert_eq!(
            event_reply_request_id(&AgentEvent::Ack { request_id: r }),
            Some(r)
        );
        let updated: AgentEvent = serde_json::from_value(serde_json::json!({
            "UsageLimitsUpdated": {
                "snapshot": {
                    "vendor": "codex", "account_key": "default", "five_hour": null, "weekly": null,
                    "model_scoped": [], "plan": null, "in_flight": true,
                    "state": { "kind": "Ready" }, "revision": 1
                },
                "subscribed": []
            }
        }))
        .unwrap();
        assert_eq!(event_reply_request_id(&updated), None);
    }
}
