# Messaging (agent mail kernel and its daemon host)

> Live reference doc — edit in place when the code moves. Anchors are file and symbol names only.
> Decisions and their reasons: ADR-0110 (kernel crate, port ownership) · ADR-0086 (dual entrance MCP + CLI) · ADR-0104 (idle gate, batch injection) · ADR-0111 (send overhaul, one path) · ADR-0116 (three entrance branches, dormant parking, reply contract failure) · ADR-0121 (`@all` vs `@here`) · ADR-0125 (send drains its own queue) · ADR-0309 (internal key = agent id, edge address = name) · ADR-0113 · ADR-0231 (busy facts). Files under `docs/decisions/`.
> Canonical crate boundary = the header of `crates/engram-dashboard-messaging/src/lib.rs`; per-module rules = each module header.

**Whole picture** = `docs/reference/architecture.md` · **Daemon link** = `docs/reference/structure/daemon-link.md`

## What it is

`engram-dashboard-messaging` is the mail kernel between agents: address resolution, per-recipient decision, parking, injection order, the idle gate, TTL, the reply contract (request / `reply_by` / overdue notice) and the ledger. It depends on no workspace crate; the daemon plugs real objects into port traits the kernel owns. `MessagingService::handle_send` is the one send entry point — single, multi-recipient and `@` group sends share it. Each send takes one decision-source snapshot (`DeliveryPort::addressing_sources`), resolves every token once from it, enqueues each accepted recipient at its queue tail, then drains live recipients' queues in the same call outside the lock. Agents reach it through two daemon entrances — MCP tool `eg_send` and HTTP `/control/send` (CLI `engram mail send`) — both funnelled into `control::ingress::handle_send`. Everything is in memory; nothing is persisted.

## Modules

Kernel (`crates/engram-dashboard-messaging/src/`):

- `lib.rs` — `PeerId` (= `uuid::Uuid`; the host `AgentId` passes unconverted) and `SenderIdentity`.
- `service.rs` — `MessagingService`: the send pipeline (resolution, per-recipient decision, park, drain), async flushes, the maintenance sweep, profile-deletion cleanup and lookups; the ports `DeliveryPort` · `ControlPlanePort` · `FlushTrigger` and the snapshot type `AddressingSources`.
- `mailbox.rs` — `Mailbox`: per-recipient FIFO parking keyed by recipient id; two independent lanes (`ParkKind::Message` capped by `MAILBOX_CAP`, `ParkKind::Notice` capped by `NOTICE_CAP`); expiry by `PARK_TTL`; in-flight accounting.
- `ledger.rs` — `Ledger`: history ring (`HISTORY_CAPACITY` delivery records), request tracking (`MAX_OPEN_REQUESTS`, strict `in_reply_to` matching, due timeouts), contract reservations.
- `groups.rs` — `GroupSource` / `BuiltinGroups`: expands `@here` (live pool) and `@all` (live + dormant pools). Expansion only.
- `busy.rs` — `BusyGate`, `BusyPolicy` (facts + `BUSY_MAX_TURN` cap, `sweep_stale_busy`), `TurnFacts`, `IdleNotifier`, `BusyReason`.
- `envelope.rs` — the single wrap point `wrap_message` / `wrap_notice`, `EnvelopeFormat`, `Entrance` (`Mcp` · `Cli` · `Daemon`), `new_msg_id`, delivery observation types.

Daemon host (`crates/engram-dashboard-daemon/src/`):

- `messaging_host.rs` — the adapters (its header is the seam ledger): `ManagerDeliveryPort` (`DeliveryPort` over `AgentManager`), `ManagerTurnFacts` (`TurnFacts`), `ControlPlanePort for ControlRegistry`, `messaging_for_manager_gated` / `busy_gate_for_manager`, `ChannelIdleNotifier` (`IdleNotifier` + `FlushTrigger` → flush channel) with `IdleCoalescer`, `MessagingFlushSink` (status-sink decorator; `RosterDiff` → `FlushMsg::Appear`, `turn_ended` / `inputs_drained` → `FlushMsg::Idle`), `spawn_flush_worker` / `run_flush_worker` / `run_flush_lane` (delivery lanes on `spawn_blocking`).
- `control/ingress.rs` — `ControlCommand`, `validate_contract` (request / `reply_by` / `reply_to` rules), `handle_send` (argument checks, `MAX_BODY_BYTES`, sender display name, one id-collision retry, then the kernel), `handle_messages` (`eg_messages` lookups), result JSON.
- `control/mcp_server.rs` — `eg_send` / `eg_messages` tools and `control_send_handler` (`/control/send`; `cli_recipient_tokens` splits commas for this route only), all calling `ingress::handle_send` on `spawn_blocking`; `MessagingSlot` (late-injected `Arc<MessagingService>`); `ControlRoute::is_mail`.
- `control/registry.rs` — `ControlRegistry` (Bearer token → `BoundIdentity`); `From<BoundIdentity> for SenderIdentity`.
- `connection_core.rs` — the only caller of `MessagingService::handle_profile_deleted` (WS `DeleteProfile`).
- `lib.rs` `run` — assembly steps 6.4–6.7: busy gate, `MessagingService`, the 60 s sweep task (`sweep` + `sweep_stale_busy`), the flush worker.

## Wiring and flow

- Send: agent → MCP `eg_send` or `/control/send` (Bearer) → `BoundIdentity` from the token, never the payload → `ControlCommand` → `ingress::handle_send`: contract validation, envelope-format support, recipient trim, single-recipient rule for `reply_to`, body cap → `MessagingService::handle_send(msg_id, SenderIdentity::from(bound), sender_name, to, body, entrance, meta)`. On `SendReject::IdCollision` ingress retries once with a new id; a second collision answers `INTERNAL_ID_COLLISION`.
- Resolution (`resolve_token`, pure, same rule as agent `resolve_in`): a UUID-shaped token is looked up as live id, then dormant id; otherwise exact case-sensitive name — live layer first, dormant layer only when no live match; two or more in a layer → `Resolution::Ambiguous` for that layer; none → `NotFound`. `@` tokens expand through `GroupSource` (sender removed by id) and expanded members resolve by id (`resolve_id`). Rows are deduplicated by resolved id, or by notation if unresolved.
- Per-row outcome: live → enqueue, then the same-call drain gives `Delivered` or `Pending`; dormant → park and return `Pending` with no drain (delivered by the appearance flush); not found → `RECIPIENT_NOT_FOUND`; ambiguous → `RECIPIENT_AMBIGUOUS` (`ambiguous_hint`: count + layer, "tell the user"); full lane → `MAILBOX_FULL`; contract table full → `REQUEST_CAPACITY`. Whole-send bounces from the kernel: `GROUP_NOT_FOUND`, `GROUP_EMPTY`, id collision; from ingress: `INVALID_SEND_ARGS`, `BODY_TOO_LARGE`. `RECIPIENT_DELETED` appears only later in lookups. Codes = `FailCode` / `SendReject`.
- Later delivery: core status sink → `MessagingFlushSink` → unbounded flush channel (`FlushMsg::Appear` from `RosterDiff` on appearance / incarnation change; `FlushMsg::Idle` on turn end or emptied input list, coalesced) → flush worker → delivery lane (`spawn_blocking`) → `flush_for` / `flush_for_agent` → `drain_queue`. The kernel's doorbell (`FlushTrigger::request_flush`) and the cap sweep (`IdleNotifier::notify_idle`) use the same channel.
- Idle gate: `LiveAgent::turn_signal == false` → inject without asking; else `BusyGate::busy_reason` (`BusyPolicy` over `ManagerTurnFacts`): busy = user inputs pending, or halted after an error, or mid-turn within `BUSY_MAX_TURN`. Unobserved = idle. Checked once per drain.
- Reply contract: `SendMeta.request` opens a contract per accepted recipient keyed `(msg_id, recipient id)` (dormant uses the profile id) before enqueue. `reply_to` closes only the replier's own contract by strict match. Overruns are found by `Ledger::due_timeouts` in `sweep`; `deliver_notice` parks a `<notice>` (`ParkKind::Notice`, `Entrance::Daemon`) to the requester's id and rings the doorbell. Missing the deadline does not close the contract.
- Lookups: `eg_messages` / `/control/messages` → `ingress::handle_messages` → `message_state` or `open_items_for` (by caller id).

## Promises no single file shows

- Phases of one send (`handle_send`): the snapshot is taken before the lock; acceptance, contract open, parking, ledger rows and the closure of an accepted reply's contract (`ReplyClosing::close_in_lock`) happen in one lock span; drains run after the lock is released.
- Reply disposition (`reply_disposition`): if any row is accepted (delivered or pending) the replier's contract closes as replied; if every row failed and one is `RECIPIENT_NOT_FOUND` it closes as `reply_failed` (`fail_reply_contract`); any other failure (`MAILBOX_FULL`, `RECIPIENT_AMBIGUOUS`) leaves it open. A new `FailCode` must be classified there.
- One snapshot per send: `addressing_sources` is called exactly once per `handle_send`; `ManagerDeliveryPort::addressing_sources` builds `roster` and `dormant` from one `manager.roster()` read — `roster` with the same predicate as `live_agents` (live + `reads_messages`, so shells are excluded), `dormant` = profiles with no live session, duplicates kept.
- Lock discipline: one `Mutex<MessagingState>` (mailbox + ledger). Never call `DeliveryPort`, `ControlPlanePort` or `FlushTrigger` while holding it; injection may take the session locks. `BusyGate::busy_reason` *is* called under it (`drain_queue` → `gate_busy_reason`), so a `BusyGate` implementation must not block and must not take the messaging lock (the `busy.rs` trait doc saying the call sites are outside is stale).
- Non-blocking notification paths, all into the unbounded flush channel: agent output pump threads call the status sink (`MessagingFlushSink::turn_ended` / `inputs_drained` → `ChannelIdleNotifier::enqueue`); the 60 s sweep task calls `IdleNotifier::notify_idle` (from `BusyPolicy::sweep_stale_busy`) and `FlushTrigger::request_flush` (from `deliver_notice`); request threads and flush-lane blocking threads call `request_flush` from drains. `RosterDiff` enqueues while holding its own lock (snapshot order = channel order).
- Order: a recipient sees enqueue order. Drains for one recipient never overlap (queue-axis in-flight + `in_flight_targets`). A drain that finds one in progress only records itself as a waiter (`deferred_flush` / `deferred_by_target`) and returns; the drain holding the in-flight receipt rings `request_flush` for each waiter when it settles.
- Keys: mailbox queue key, contract key and ledger record key all use the agent id (`PeerId`), never the name. `msg_id` is unique per daemon only (`new_msg_id`, collision-checked against this daemon's ledger). Names exist only at the edge (tokens, envelope `from` / `to`, response `to`).
- Mailbox lanes are independent: message backlog never blocks notices; the notice lane reclaims its oldest notice on overflow and never bounces.
- `Entrance` is a log / observation label only; the pipeline does not branch on it.
- The kernel trusts `SendMeta` as validated: ingress is the only validator (`handle_send` has debug asserts only).
- Who fills what: the daemon fills `AddressingSources` and `LiveAgent::turn_signal`, the envelope format (`ControlRegistry`), turn facts (`ManagerTurnFacts`) and the sender display name (`ingress::sender_display_name`, byte-equal to the roster name). The kernel fills results, hints, ledger rows and notices.
- Roster-change notice is not a messaging input: the composition notice `RosterChanged` (agent crate port, `crates/engram-dashboard-agent/src/commands.rs`) → `control::commands::LateRosterBroadcast` → `control::agent::RosterBroadcast` → `connection_core::RosterFanout` goes to the shells only, and only for command-table verbs; WS-originated profile changes broadcast directly from `connection_core` (`broadcast_profile_list`). Messaging learns about agents only from its per-send snapshot and from `MessagingFlushSink::agent_list_updated` (appearance / incarnation diff by id).
- The control entrance and mail resolve targets by the same rule; `crates/engram-dashboard-daemon/tests/control_agent.rs` (`resolver_alignment`) guards it.

## Where to look

- Send pipeline: `crates/engram-dashboard-messaging/src/service.rs` module header, `MessagingService::handle_send`, `resolve_addressing`, `resolve_token`, `admission`, `reply_disposition`, `drain_queue`, `sweep`, `deliver_notice`.
- Ports: `service.rs` (`DeliveryPort`, `ControlPlanePort`, `FlushTrigger`, `AddressingSources`), `busy.rs` (`BusyGate`, `TurnFacts`, `IdleNotifier`), `groups.rs` (`GroupSource`).
- Storage: `mailbox.rs` (`Mailbox`, `ParkKind`), `ledger.rs` (`Ledger`, `DeliveryStatus`).
- Envelope and entrances: `envelope.rs` (`wrap_message`, `wrap_notice`, `Entrance`).
- Host: `crates/engram-dashboard-daemon/src/messaging_host.rs` (header seam ledger), `control/ingress.rs` (`handle_send`, `validate_contract`), `control/mcp_server.rs` (`eg_send`, `control_send_handler`, `cli_recipient_tokens`, `MessagingSlot`), `control/registry.rs` (`BoundIdentity`), `lib.rs` `run` (steps 6.4–6.7).
- Tests: kernel unit tests in `service.rs` / `mailbox.rs` / `ledger.rs`; daemon `tests/control_send.rs`, `tests/mail_gate.rs`, `tests/mcp_control.rs`, `tests/control_agent.rs`.
