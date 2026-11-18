# Messaging (agent mail kernel and its daemon host)

> Live reference doc — edit in place when the code moves. Anchors are file and symbol names only.
> Decisions and their reasons: ADR-0110 (kernel crate, port ownership) · ADR-0086 (dual entrance MCP + CLI) · ADR-0104 (idle gate, batch injection) · ADR-0111 (send overhaul, one path) · ADR-0116 (three entrance branches, dormant parking, reply contract failure) · ADR-0121 (`@all` vs `@here`) · ADR-0125 (send drains its own queue) · ADR-0309 (internal key = agent id, edge address = name) · ADR-0113 · ADR-0231 (busy facts). Files under `docs/decisions/`.
> Canonical crate boundary = the header of `crates/engram-dashboard-messaging/src/lib.rs`; per-module rules = each module header.

**Whole picture** = `docs/reference/architecture.md` · **Daemon link** = `docs/reference/structure/daemon-link.md`

## What it is

`engram-dashboard-messaging` is the mail kernel between agents: address resolution, per-recipient decision, parking, injection order, the idle gate, TTL, the reply contract (request / `reply_by` / overdue notice) and the ledger. It depends on no workspace crate (external: `uuid`, `tracing`); the daemon plugs real objects into port traits the kernel owns. `MessagingService::handle_send` is the one send entry point — single, multi-recipient and `@` group sends share it. Each send takes one decision-source snapshot (`DeliveryPort::addressing_sources`), resolves every token once from it, enqueues each accepted recipient at its queue tail, then drains that recipient's queue in the same call outside the lock. Agents reach it through two daemon entrances — MCP tool `eg_send` and HTTP `/control/send` (CLI `engram mail send`) — both funnelled into `control::ingress::handle_send`. Everything is in memory; nothing is persisted.

## Modules

Kernel (`crates/engram-dashboard-messaging/src/`):

- `lib.rs` — `PeerId` (= `uuid::Uuid`, the host `AgentId` passes unconverted), `SenderIdentity` (`peer_id` + `epoch`, `Copy`).
- `service.rs` — `MessagingService` (`new` / `new_gated` / `with_flush_trigger`, `handle_send`, `flush_for`, `flush_for_agent`, `sweep`, `handle_profile_deleted`, `message_state`, `open_items_for`); ports `DeliveryPort`, `ControlPlanePort`, `FlushTrigger`; snapshot types `AddressingSources`, `LiveAgent`, `DormantAgent`; result types `RecipientResult`, `SendStatus`, `FailCode`, `SendReject`, `SendMeta`, `InjectReceipt`; private resolution (`resolve_addressing`, `resolve_token`, `resolve_id`, `Resolution`, `NameLayer`, `push_recipient`), `admission`, `drain_queue`, `park_into`, `park_notice`, `deliver_notice`, hint builders (`ambiguous_hint`, `park_hint_*`, `mailbox_full_hint`).
- `mailbox.rs` — `Mailbox`: per-recipient FIFO parking keyed by recipient id; `ParkKind::Message` lane (`MAILBOX_CAP` 100) and `ParkKind::Notice` lane (`NOTICE_CAP` 64); `PARK_TTL` 24 h; in-flight accounting (`take_in_flight` / `FlightTicket`); `admission_seq` ordering.
- `ledger.rs` — `Ledger`: history ring (`HISTORY_CAPACITY` 4096 delivery records, `MessageRecord`, `DeliveryStatus`), request tracking (`MAX_OPEN_REQUESTS` 512, `DueTimeout`, strict `in_reply_to` matching), contract reservations.
- `groups.rs` — `GroupSource` / `BuiltinGroups` (`@here` = live pool, `@all` = live + dormant pools), `MemberPools`, `GroupMember`, `normalize_group_name`, `GroupError`. Expansion only.
- `busy.rs` — `BusyGate` (`busy_reason`), `BusyPolicy` (facts + `BUSY_MAX_TURN` 30 min cap, `sweep_stale_busy`), `TurnFacts` / `TurnFact`, `IdleNotifier`, `BusyReason`, `AlwaysIdleGate`.
- `envelope.rs` — the single wrap point `wrap_message` / `wrap_notice`, `EnvelopeFormat`, `EnvelopeFields`, `Entrance` (`Mcp` · `Cli` · `Daemon`), `new_msg_id`, `DeliveryObservation` / `DeliveryObserver`.

Daemon host (`crates/engram-dashboard-daemon/src/`):

- `messaging_host.rs` — the adapters (its header is the seam ledger): `ManagerDeliveryPort` (`DeliveryPort` over `AgentManager`: `inject`, `live_agents`, `addressing_sources`, `is_agent_live`), `is_live`, `ManagerTurnFacts` (`TurnFacts`), `ControlPlanePort for ControlRegistry`, `messaging_for_manager_gated` / `busy_gate_for_manager`, `ChannelIdleNotifier` (`IdleNotifier` + `FlushTrigger` → flush channel) with `IdleCoalescer`, `MessagingFlushSink` (status-sink decorator; `RosterDiff` → `FlushMsg::Appear`, `turn_ended` / `inputs_drained` → `FlushMsg::Idle`), `spawn_flush_worker` / `run_flush_worker` / `run_flush_lane` (delivery lanes, `spawn_blocking`).
- `control/ingress.rs` — `ControlCommand`, `SendContract`, `validate_contract` (request / `reply_by` / `reply_to` rules, `MIN_REPLY_BY_SECS`, `MAX_REPLY_BY_SECS`), `handle_send` (argument checks, `MAX_BODY_BYTES` 64 KiB, sender display name, then the kernel), `handle_messages` (`eg_messages` lookups), `ControlResult` / `ControlQueryResult` JSON.
- `control/mcp_server.rs` — `eg_send` / `eg_messages` tools and `control_send_handler` (`/control/send`), both calling `ingress::handle_send` on `spawn_blocking`; `MessagingSlot` (late-injected `Arc<MessagingService>`); `ControlRoute::is_mail` (mail routes gated by credential).
- `control/registry.rs` — `ControlRegistry` (Bearer token → `BoundIdentity`); `From<BoundIdentity> for SenderIdentity`.
- `connection_core.rs` — the only caller of `handle_profile_deleted` (WS `DeleteProfile`).
- `lib.rs` `run` — assembly steps 6.4–6.7: busy gate, `MessagingService`, the 60 s sweep task (`sweep` + `sweep_stale_busy`), the flush worker.

## Wiring and flow

- Send: agent → MCP `eg_send` or `/control/send` (Bearer) → `BoundIdentity` from the token, never the payload → `ControlCommand` → `ingress::handle_send(manager, registry, messaging, Entrance, cmd)`: `validate_contract`, envelope-format support, trim recipients (CLI splits commas before), single-recipient rule for `reply_to`, body cap → `MessagingService::handle_send(msg_id, SenderIdentity::from(bound), sender_name, to, body, entrance, meta)`.
- Inside `handle_send`: ① `addressing_sources()` once (outside the lock) ② `resolve_addressing`: `@` tokens expand through `GroupSource` (sender removed by id), other tokens go through `resolve_token`, expanded members through `resolve_id`; rows deduplicated by resolved id (or notation if unresolved) ③ under the lock: id collision check (the first side effect), per-row `admission`, reply-contract open (request) before enqueue, park every accepted row, ledger rows, and the replier's contract closed in the same lock span when a `reply_to` row is accepted (`ReplyClosing::close_in_lock`) ④ outside the lock: drain each accepted recipient's queue (`drain_queue` — busy gate checked once per drain; inject via `DeliveryPort::inject`); a `reply_to` whose row failed closes the contract as `reply_failed` (`fail_reply_contract`) ⑤ return one `RecipientResult` per row or a `SendReject`.
- Resolution (`resolve_token`, pure, same rule as agent `resolve_in`): UUID-shaped token → live id, then dormant id; else exact case-sensitive name: one live → `Resolution::Live`; two+ live → `Ambiguous { layer: Live }`; no live and one dormant → `Dormant`; two+ dormant → `Ambiguous { layer: Saved }`; none → `NotFound`. A live hit hides the dormant layer.
- Per-row outcome: `Live` → enqueue + same-call drain → `Delivered` or `Pending` (busy, overlapping drain, inject failure); `Dormant` → park `Pending` until the agent appears; `NotFound` → `Failed` + `RECIPIENT_NOT_FOUND`; `Ambiguous` → `Failed` + `RECIPIENT_AMBIGUOUS` with `ambiguous_hint` (count + layer, "tell the user"); full lane → `MAILBOX_FULL`; contract table full → `REQUEST_CAPACITY`. Whole-send bounces: `GROUP_NOT_FOUND`, `GROUP_EMPTY`, id collision; entrance-only bounces: `INVALID_SEND_ARGS`, `BODY_TOO_LARGE`. `RECIPIENT_DELETED` appears only later in lookups.
- Later delivery: core status sink → `MessagingFlushSink` → unbounded flush channel (`FlushMsg::Appear` from `RosterDiff` on appearance / incarnation change; `FlushMsg::Idle` on turn end or emptied input list, coalesced) → flush worker → delivery lane (`spawn_blocking`) → `MessagingService::flush_for` / `flush_for_agent` → `drain_queue`. The kernel's own doorbell (`FlushTrigger::request_flush`) uses the same channel.
- Idle gate: `LiveAgent::turn_signal == false` → inject without asking; else `BusyGate::busy_reason` (`BusyPolicy` over `ManagerTurnFacts`): busy = user inputs pending, or halted after an error, or mid-turn within `BUSY_MAX_TURN`. Unobserved = idle. The 60 s sweep wakes turns past the cap through `IdleNotifier`.
- Reply contract: `SendMeta.request` opens a contract per accepted recipient keyed `(msg_id, recipient id)` (dormant uses the profile id); `reply_to` closes only the replier's own contract by strict match; `reply_by` overruns are found by `Ledger::due_timeouts` in `sweep`, and `deliver_notice` parks a `<notice>` (`ParkKind::Notice`, sender label `engram`, `Entrance::Daemon`) to the requester's id and rings the doorbell — it never injects on the sweep task. Missing the deadline does not close the contract.
- Lookups: `eg_messages` / `/control/messages` → `ingress::handle_messages` → `message_state(msg_id)` or `open_items_for(caller id)`.

## Promises no single file shows

- One snapshot per send: `addressing_sources` is called exactly once per `handle_send`; the adapter builds `roster` and `dormant` from one manager roster read (`ManagerDeliveryPort::addressing_sources`), `roster` with the same predicate as `live_agents` (live + `reads_messages`, so shells are excluded), `dormant` = profiles with no live session, duplicates kept.
- Lock discipline: one `Mutex<MessagingState>` (mailbox + ledger). Never call `DeliveryPort`, `ControlPlanePort` or `FlushTrigger` while holding it; injection may take the session locks. `BusyGate` is safe under it but is called outside today.
- Non-blocking callbacks: `IdleNotifier::notify_idle` runs on agent output pump threads and `FlushTrigger::request_flush` on request threads — both only enqueue onto the unbounded flush channel. `RosterDiff` enqueues while holding its own lock (snapshot order = channel order).
- Order: a recipient sees enqueue order. Drains for one recipient never overlap (queue-axis in-flight + `in_flight_targets`); a backed-off drain re-rings the doorbell instead of losing the wakeup.
- Keys: mailbox queue key, contract key and ledger record key all use the agent id (`PeerId`), never the name. `msg_id` is unique per daemon only (`new_msg_id`, `m-` + 8 base-36 chars, collision-checked against this daemon's ledger). Names exist only at the edge (tokens, envelope `from` / `to`, response `to`).
- Mailbox lanes are independent: message backlog never blocks notices; the notice lane reclaims its oldest notice on overflow and never bounces.
- `Entrance` is a log / observation label only; the pipeline does not branch on it.
- The kernel trusts `SendMeta` as validated: ingress is the only validator (`handle_send` has debug asserts only).
- Who fills what: the daemon fills `AddressingSources` and `LiveAgent::turn_signal` (`messaging_host.rs`), the envelope format (`ControlRegistry`), turn facts (`ManagerTurnFacts`) and the sender display name (`ingress::sender_display_name`, byte-equal to the roster name). The kernel fills results, hints, ledger rows and notices.
- Roster-change notice is not a messaging input: the composition notice `RosterChanged` (agent crate port, `crates/engram-dashboard-agent/src/commands.rs`) → `control::commands::LateRosterBroadcast` → `control::agent::RosterBroadcast` → `connection_core::RosterFanout` goes to the shells only, and only for command-table verbs; WS-originated profile changes broadcast directly from `connection_core` (`broadcast_profile_list`). Messaging learns about agents only from its per-send snapshot and from `MessagingFlushSink::agent_list_updated` (appearance / incarnation diff by id).
- The control entrance and mail resolve targets by the same rule; `crates/engram-dashboard-daemon/tests/control_agent.rs` (`resolver_alignment`) guards it.

## Where to look

- Send pipeline: `crates/engram-dashboard-messaging/src/service.rs` (`MessagingService::handle_send`, `resolve_addressing`, `resolve_token`, `admission`, `drain_queue`, `sweep`, `deliver_notice`), its module header.
- Ports: `service.rs` (`DeliveryPort`, `ControlPlanePort`, `FlushTrigger`, `AddressingSources`), `busy.rs` (`BusyGate`, `TurnFacts`, `IdleNotifier`), `groups.rs` (`GroupSource`).
- Storage: `mailbox.rs` (`Mailbox`, `ParkKind`), `ledger.rs` (`Ledger`, `DeliveryStatus`).
- Envelope and entrances: `envelope.rs` (`wrap_message`, `wrap_notice`, `Entrance`).
- Host: `crates/engram-dashboard-daemon/src/messaging_host.rs` (header seam ledger), `control/ingress.rs` (`handle_send`, `validate_contract`), `control/mcp_server.rs` (`eg_send`, `control_send_handler`, `MessagingSlot`), `control/registry.rs` (`BoundIdentity`), `lib.rs` `run` (steps 6.4–6.7).
- Tests: kernel unit tests in `service.rs` / `mailbox.rs` / `ledger.rs`; daemon `tests/control_send.rs`, `tests/mail_gate.rs`, `tests/mcp_control.rs`, `tests/control_agent.rs`.
