# TRD — Client/backend path unification (ADR-0020 implementation)

Decisions and rejected alternatives are in ADR-0020. This document is the **how** (stages, interfaces, tests). 4 stages, each stage = coder (opus) → reviewer-deep → QA (build/test+cdp) → commit after the gate. Keep the WS tests green at every stage (behavior-preserving), and update unit tests cumulatively.

## Cut line summary
```
바이트/소켓 만지는 모든 것 = 어댑터 (carrier별)
AgentCommand → ConnectionCore.dispatch → AgentManager → Outbound = core (공유)
```

## Stage 1 — Backend: extract ConnectionCore (behavior-preserving, keep WS green)
**Goal:** Move the dispatch logic of `ws.rs` out into a transport-neutral ConnectionCore with zero behavior change. The WS adapter runs on top of it.

- **New traits/types (daemon crate):**
  - `trait OutboundSink { fn enqueue(&self, out: Outbound) -> Result<(), SinkError>; }` — abstraction for sending responses/events/output.
  - `enum Outbound { Event(AgentEvent), Binary(Vec<u8>), Close }` — carrier-neutral (the superordinate concept of WsOutbound). Encoding (JSON/frame) is owned by the sink implementation.
  - `struct ConnectionSession { conn_id, subs: HashMap<AgentId,SinkId>, owned_viewports: Vec<(AgentId,String)> }` — per-connection lifetime state.
  - `struct ConnectionCore { manager, multiview }` — `async fn dispatch(&self, cmd, session, sink: &dyn OutboundSink) -> DispatchFlow`. `enum DispatchFlow { Continue, Close }` (replaces the current bool).
- **What moves (ws.rs→connection_core.rs):** all dispatch match arms, handle_subscribe (TOCTOU-safe), reply/send_error/event_json (changed to go through Outbound), the *_to_wire conversions, MultiViewState.
- **WS adapter (stays in handle_connection):** handshake/auth/Origin/protocol_version, ws.split, read_task (frame→cmd), write_task (Outbound→Message), keepalive, close_signal. `WsOutboundSink` implements `OutboundSink` (pushes to conn_tx, encodes AgentEvent→JSON text/binary frame, on queue saturation SinkError→close_signal).
- **Unit tests:** drive ConnectionCore directly with a `MockOutboundSink` (records into a Vec) — the R1 [Ack,Binary,ReplayComplete] order, manager calls per dispatch arm. The existing ws_e2e 44 stay green as is.
- **Gate:** ws_e2e green, no regression in core/protocol, clippy0, `rg "use tauri" core`=0.

## Stage 2 — Backend: Tauri adapter (embedded over the protocol)
**Goal:** Make the local side also speak AgentCommand. 20 invokes → 1 generic one.

- `#[tauri::command] async fn agent_command(state, cmd: AgentCommand)` — enqueues onto the state's per-conn **inbound mpsc**. A **single command loop task** takes them out in order and calls `ConnectionCore.dispatch` (★decision 2 reinforcement: serializes invoke racing★).
- `TauriOutboundSink` implements `OutboundSink` — `Outbound::Event`→Channel emit, `Outbound::Binary`→base64 PtyEvent Channel (keep the existing encoding). For the output subscription, either absorb the existing subscribe_agent_output path into ConnectionCore.Subscribe or route through the sink.
- src-tauri setup: register one ConnectionCore + one conn_id (single client) + start the command loop.
- **Unit/integration:** embedded conformance — drive the same ConnectionCore with the Tauri sink, assert that the same commands as WS give the same manager effects. Racing test (N parallel invokes → order preserved).
- **Gate:** the above + the existing src-tauri build.

## Stage 3 — Frontend: ProtocolClient over Transport
**Goal:** one client + two transports.

- `interface Transport { send(payload): void; onMessage(cb); readonly state; onStateChange(cb); close() }`.
- `WsTransport` (the WS part extracted from DaemonClient: openSocket/Auth/Hello/scheduleReconnect/ws.send·onmessage) / `InProcTransport` (invoke('agent_command',{cmd}) + Channel receive, state always connected, reconnect is a no-op).
- `ProtocolClient implements AgentClient` — holds the request_id pending map, seq high-water dedup, epoch guard and resubscribe resume independent of the transport (DaemonClient logic promoted). Under InProc, dedup/reconnect converge to harmless no-ops.
- `clientFactory`: mode → pick transport → `new ProtocolClient(transport)`. The EmbeddedClient/DaemonClient classes are retired.
- **vitest update:** with a transport mock — dedup (replay_from≠high-water), epoch reset, resume, request_id matching. Convergence to no-ops with an InProc mock. #13133/order preservation.
- **Gate:** vitest green, tsc0.

## Stage 4 — Delete the old path + measure
- Delete/shrink: the old EmbeddedClient invoke path, ptyApi (or absorb it into InProcTransport), the old Tauri commands (agent/profile/pty dispatch — replaced by agent_command). Keep non-agent commands such as discovery.
- **QA cdp measurement (required):** for each of the embedded and daemon modes, measure the full path spawn→write echo→interrupt→kill→subscribe replay→profile CRUD in WebView2 (code green ≠ working).
- **Gate:** full tests + cdp PASS → update the step-log/ADR index.

## Regression guards (ADR-0020 R1~R7)
R1 FIFO · R2 dedup · R3 resume (check baseline coverage) · R4 finalize once · R5 kill two verbs · R6 close_signal (stays in the adapter) · R7 core isolation. Assert the relevant R in each stage's QA.
