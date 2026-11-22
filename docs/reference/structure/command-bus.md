# Command bus

> Live reference doc — edit in place when the code moves. Anchors are file and symbol names only.
> Decisions and their reasons: ADR-0155 (one bus; declarations next to producers; same three delivery steps per hop; registry by runtime registration only) · ADR-0156 (`help` blob opaque) · ADR-0154 (daemon picks the target from its own registry) · ADR-0157 (unknown fields: entrances refuse, wiring tolerates) · ADR-0160 (agent relay through the control routes) · ADR-0161 (caller request ids) · ADR-0081 (opaque relay, apply off the connection task) · ADR-0308 (daemon rewrites agent-reference args to ids) · ADR-0231 (input lease) · ADR-0055 (single webview registry). Files under `docs/decisions/`.
> Canonical crate contract = the header of `crates/engram-dashboard-command/src/lib.rs`. Rules for designing LLM-facing commands = `docs/reference/llm-command-conventions.md`.

**Whole picture** = `docs/reference/architecture.md` · **Daemon link** = `docs/reference/structure/daemon-link.md`

## What it is

One command vocabulary shared by every process. Owners (who answer names) are the daemon (its own table), the shell and the webview. Callers today are the shell (`DaemonClient::send_command` issues `AgentCommand::Command`) and agents through the `engram` CLI's control routes; agents can only call, never own (`CommandBus::invoke` doc), and the webview takes part only as an owner (it issues no `AgentCommand::Command` and has no `CommandReply` branch — `AgentEvent::CommandReply` doc in protocol). `engram-dashboard-command` is tooling only — envelope, error vocabulary, declaration macro, per-process table, owner registry, the three-step `route` — with zero workspace dependencies and zero commands. Commands are declared with `declare_commands!` next to the code that runs them, and each process builds its own `CommandTable` from those declarations plus injected handlers (`make_table(deps)`). Delivery is the same at every hop: in my table → in the registry (an owner registered at runtime) → error. The daemon hosts the registry (`CommandRoster`) and the delivery machine (`command_delivery`); owners register their names over their connection (`RegisterCommands` / `UpdateCommands`). Today only two crates declare commands: agent (`agent.*`, `usage.*`) and the shell (`window.*` · `tab.*` · `slot.*` · `split.*` · `layout.*` · `settings.*` · `restore.*` · `agent.spawnInto` and the rest of `src-tauri/src/layout/commands.rs`). The daemon declares none — it executes agent's table. Webview commands live in the frontend registry and reach the bus through the shell.

## Modules

Tooling (`crates/engram-dashboard-command/src/`):

- `lib.rs` — re-exports and the three invariants (zero workspace deps · zero commands · linker collection stops at `CommandSpec`).
- `macros.rs` — `declare_commands!`: argument / return structs, `CommandSpec` statics (linker-collected via `inventory`), JSON Schema text, the declaring crate's `CATALOG_VERSION`; `transmitted`.
- `spec.rs` — `CommandSpec`, `Effect`, `CommandDecl`, `LinkedSpec`, `command_specs`, `spec_of`, `duplicate_command_names`, `lint_spec`, `spec_item_json`, `catalog_json`, `COMMON_ERRORS`.
- `table.rs` — `CommandTable` (`new` · `insert` · `call` · `check_args` · `contains` · `decls` · `specs`), `CommandHandler`, `blocking_handler`, `CommandFuture`, `TableError`.
- `coerce.rs` — schema-driven coercion run once inside `CommandTable::call`.
- `roster.rs` — `Roster` (owner registry: `register` full last-wins, `update` diff, `disconnect`, `lookup`, `agent_args`), `RosterEntry`, `OwnerLookup`, `OwnerLookupSource`.
- `route.rs` — `route`: the three delivery steps, panic net (`guard_panic`, `OUTCOME_UNKNOWN` while forwarding).
- `link.rs` — `CommandLink` (send an envelope, await the same `request_id`), `InboundCommands` (`on_command` must return at once), `ReplySink` (exactly one reply, enforced by `Drop`).
- `envelope.rs` — `CommandEnvelope`, `CommandReply`, `RequestId`, `OwnerToken`.
- `error.rs` — `CommandError`, `ErrorCode`, `RetryMode`.
- `testing.rs` (feature `test-support`) — harness, `block_on`.

Daemon (`crates/engram-dashboard-daemon/src/`):

- `command_roster.rs` — `CommandRoster`: the shared handle to the tool `Roster` plus connection lifetime (`attach` / `detach`, `sink_of` owner token → connection frame outlet, refusal of late packets from a dropped connection).
- `command_delivery.rs` — `Caller` (the typed caller of the daemon table: `Shell(ConnId)` = a live shell connection, the human path; `Agent(BoundIdentity)` = an agent token — ADR-0394), `CommandBus` (`new` returns it with its `BusSweeper`; `roster` · `deliveries` · `locals` · `invoke`), `deliver` (the socket entrance, caller = that shell connection) and `deliver_as` (the relay door's, caller = the asking agent), `CommandDeliveries` (seat table: `request_id` → origin connection + deadline; `open` / `complete` / `release` / `expire` / `drop_origin`), `LocalCommands` port (stage 1: `claim` · `run` · `decls` · `agent_roster`), `OwnerLink` (`CommandLink` that writes to the owner's connection), `resolve_agent_args` (ADR-0308), `spawn_local`, `retains_the_id`.
- `control/commands.rs` — `make_daemon_table` (builds the daemon's table from agent `make_table` with the real ports), `DaemonTable` (the table plus what the input-lease check needs, plugged as one value; `with_peer` joins the daemon-declared `peer.*` table), `call_daemon_command` (the only caller of the tables, with a typed `Caller`: membership → `check_args` → for `agent.*`·`usage.*` the `admit_input` lease check, for `peer.*` `PeerCommands::call` — identity on first need, then the approval gate → blocking drive), `peer_caller` (the typed caller as the peer gate sees it), `DaemonLocalCommands` (`LocalCommands` over `CommandTableSlot`), `InputLease`, `NoInputLeases`, `NoUsageLimits`.
- `link/peer/commands.rs` — the `peer.*` declarations (`declare_commands!`), `PeerCommands` (its table is private; `call` runs the approval gate before any handler); approvals and the ask-the-user seam = `docs/reference/structure/peer-link.md`.
- `control/catalog.rs` — `/control/commands` (`handle_list`: daemon table ∪ registry, `callable` flag) and `/control/call` (`handle_call` → `CallOutcome::Answered` or `CallOutcome::Relay`).
- `control/agent.rs` — `/control/agent`: `{verb}` → `agent.<verb>` → `call_daemon_command`; `RosterBroadcast` port.
- `control/mcp_server.rs` — HTTP handlers for those routes (`spawn_blocking` for table calls; relay via `CommandBus::invoke`), `CommandTableSlot`.
- `connection_core.rs` — WS dispatch of `RegisterCommands` / `UpdateCommands` / `ListCommands` / `Command` / `CommandOutcome`.
- `lib.rs` `run` — builds the one `CommandBus` before the MCP server, fills `CommandTableSlot` after the manager exists.

Shell and frontend (only the wiring):

- `src-tauri/src/layout/commands.rs` — the shell's `declare_commands!` block and handlers (`LayoutPorts`); bodies land on the same apply service as human clicks.
- `src-tauri/src/daemon_client/inbound.rs` — `InboundReceiver` (`InboundCommands`: shell table → `ViewHop` → `UNKNOWN_COMMAND`), `registration_command`; `connection.rs` `register_commands` sends `RegisterCommands` on every connect.
- `src-tauri/src/view_commands.rs` — webview hop: boot report → registration, envelope → Tauri event `command:request` to one window, outcome correlation, `reserved_names`.
- `src/commands/registry.ts` (`register`, `run`, `list`), `src/commands/viewCommandBridge.ts` (`report_view_commands`, `command:request` → `registry.run` → `report_command_outcome`; only commands with `help` are offered), `src/store/eventBus.ts` (`window.__engramCmd` = `{ list, run }` over the same registry).
- `crates/engram-dashboard-agent/src/commands.rs` — `agent.*` / `usage.*` declarations, `make_table`, ports `AgentCommandHost` · `RosterChanged` · `UsageCommandHost`, the resolver `resolve_in`.

## Wiring and flow

- Daemon assembly (`lib.rs` `run`): `CommandBus::new(CommandRoster::new(), CommandDeliveries::new(), DaemonLocalCommands::new(command_table_slot))` → the same bus copy goes to the MCP server (discovery + call) and to `run_accept_loop` (socket dispatch) → after the manager is built, `command_table_slot.set(make_daemon_table(manager, roster_broadcast_slot, multiview, usage).with_peer(PeerCommands::new(approvals)))` (the test server and the smoke bins do not join `peer.*`).
- Registration: on each connect the shell sends `AgentCommand::RegisterCommands { owner, decls, catalog_version }` (shell table + the webview commands it reported); webview changes go as `UpdateCommands` diffs. `connection_core` feeds them to `CommandRoster`; a disconnect erases that owner's names.
- Socket entrance (the shell calls a command): `AgentCommand::Command { envelope }` → `connection_core` spawns `command_delivery::deliver(roster, deliveries, locals, origin, envelope, ENTRANCE_SOCKET)` → `LocalCommands::claim` (before the seat, so the "mine" fact is born with it — `Pending::local`) → `CommandDeliveries::open` → stage 1 `LocalCommands::run` (daemon table on the blocking pool, `spawn_local`) → else `resolve_agent_args` → `route` with an empty table → registry lookup → `OwnerLink::send` writes `AgentEvent::CommandRequest` to the owner's connection → owner replies `AgentCommand::CommandOutcome { reply }` → `CommandDeliveries::complete` → `AgentEvent::CommandReply` to the origin connection → seat released after the frame went out.
- Control entrances (agents via `engram`): `/control/agent` → `call_daemon_command(table, "agent.<verb>", …, Caller::Agent(identity))`; `/control/call` → `handle_call(…, identity)`: own table → `Answered` (`Caller::Agent(identity)`); someone else's name with an owner → `Relay` → `CommandBus::invoke(request_id, name, args, identity)` (a temporary caller connection counted down from `u64::MAX`, same machine through `deliver_as` with `Caller::Agent(identity)`); `/control/commands` → `handle_list`. MCP `/mcp` has no command tool.
- Shell as owner: `CommandRequest` → `connection.rs` `accept_inbound` → `InboundReceiver::on_command` (queues, returns at once) → spawned task runs `route` over the shell table, then `ViewHop` → `view_commands` emits `command:request` to one chosen window → `viewCommandBridge.ts` `registry.run` → `report_command_outcome` → `CommandOutcome` back on the generation the request came on.
- `window.__engramCmd.run` calls the frontend registry directly (no bus); the bus reaches the same `registry.run` through `viewCommandBridge`.

## Promises no single file shows

- Zero commands in the tooling crate; a `declare_commands!` call in `crates/engram-dashboard-command/` is a violation. One declaration block per module; the shell keeps all its names in one block (one registration packet, one generation number).
- Linker collection holds only `CommandSpec`; handlers are injected at assembly (`make_table(deps)`), so tests plug fakes.
- Exactly one reply per received query on the socket entrance (`command_delivery::deliver`): the seat is opened before any stage runs and released only after the reply frame went out; every reply rests on a seat decision. A stage-1 command whose effect was applied keeps its id seat for one retention window from completion (`CommandDeliveries::settle_local`, `retains_the_id`), subject to oldest-first eviction past `MAX_RETAINED_LOCAL`; a same-id retry inside that window is not applied twice.
- That double-apply protection does not hold on the control routes: on `/control/agent` and `/control/call` the daemon's own names are answered by `call_daemon_command` / `catalog::handle_call` without the seat table, and relayed names release their seat on reply, so a retry after completion applies again (`control/catalog.rs` `CallRequest` doc, "known gap").
- `call_daemon_command` is the only door to the daemon table; it runs the entrance argument check (ADR-0157), the input-lease check (`INPUT_AFFECTING`, ADR-0231) and, for `peer.*`, the approval gate from the typed caller (ADR-0394 — never an argument or the entrance label). `route` itself never checks arguments (inter-hop tolerance), so args under someone else's name are checked nowhere on the relay path.
- Daemon table handlers are blocking and must finish on the first poll (`blocking_handler`); an async handler surfaces as `OUTCOME_UNKNOWN`. Callers run them on the blocking pool.
- `InboundCommands::on_command` must return at once on every hop (shell, daemon, webview); applying inline on the connection task self-deadlocks composite commands.
- The registry lock is never held across a send (`OwnerLookupSource` single lookup; `CommandRoster` locks only inside its file). The roster and the seat table are separate tables with separate lifetimes; do not merge them.
- Connection ids are never reused (`ConnRegistry::alloc_id` counts up from 1, `CommandBus::open_caller` down from `u64::MAX`); a seat has no generation and relies on that.
- Slots, not values: `CommandTableSlot` and `RosterBroadcastSlot` are read at call time because the table and the roster fan-out are assembled after the routes start. An empty table slot means stage 1 misses, `/control/agent` and `/control/call` answer 503, and `/control/commands` answers with the registry half only (`control_commands_handler`, `catalog::handle_list`).
- `LocalCommands::claim` is the single authority on "is this the daemon's own name", read in four places: `deliver` (stage-1 marker on the seat), `catalog::handle_call` (`Answered` vs `Relay`), `CommandBus::invoke` (refuses own names), and `ConnectionCore::refuse_names_i_answer` (bounces a whole `RegisterCommands` / `UpdateCommands` that carries any name the daemon answers). `DaemonLocalCommands::claim` also returns the declared `Effect`, which `retains_the_id` uses: a verb declared `Read` whose body writes silently loses double-apply protection.
- The daemon declares one table of its own, `peer.*` (`link/peer/commands.rs`), joined by `DaemonTable::with_peer` behind `call_daemon_command`. It is visible through `DaemonTable::specs` / `decls` (discovery, `claim`) and through the same `contains` / `check_args` / `call` that `call_daemon_command` uses — anything joined later must be too, or the four `claim` readers and the executing table disagree.
- Names the daemon answers are filtered by no one on the webview side; a webview `help` on such a name makes the daemon bounce the whole registration (ADR-0270).

## Where to look

- Tooling: `crates/engram-dashboard-command/src/lib.rs` header, `route.rs` (`route`), `table.rs` (`CommandTable::call`, `check_args`), `roster.rs` (`Roster`), `link.rs` (`InboundCommands`, `ReplySink`), `macros.rs` (`declare_commands!`).
- Daemon delivery: `crates/engram-dashboard-daemon/src/command_delivery.rs` header and `deliver`, `CommandBus::invoke`, `CommandDeliveries`, `LocalCommands`; `command_roster.rs` (`CommandRoster`).
- Daemon table: `crates/engram-dashboard-daemon/src/control/commands.rs` (`make_daemon_table`, `DaemonTable`, `call_daemon_command`, `admit_input`, `DaemonLocalCommands`); routes `control/catalog.rs` (`handle_call`, `handle_list`), `control/agent.rs`, `control/mcp_server.rs`.
- Declarations: `crates/engram-dashboard-agent/src/commands.rs` (`declare_commands!`, `make_table`), `src-tauri/src/layout/commands.rs`.
- Shell and webview hop: `src-tauri/src/daemon_client/inbound.rs` (`InboundReceiver`, `registration_command`), `src-tauri/src/daemon_client/connection.rs` (`register_commands`), `src-tauri/src/view_commands.rs`, `src/commands/viewCommandBridge.ts`, `src/commands/registry.ts`, `src/store/eventBus.ts`.
- Wire: `crates/engram-dashboard-protocol/src/messages.rs` (`AgentCommand::RegisterCommands` · `UpdateCommands` · `ListCommands` · `Command` · `CommandOutcome`, `AgentEvent::CommandRequest` · `CommandReply` · `CommandList`).
