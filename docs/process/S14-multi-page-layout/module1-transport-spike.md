# Module ① transport relay unification — implementation design spike (ADR-0036)

**Status:** T1~T4 implemented / **T5~T8 before coding — D1~D5 decided + research reflected (§7)**
**Written:** 2026-06-27 (dashboard2, opus Plan agent spike — based on checking the actual code) · **§7 added 2026-06-28** (carrier/fan-out research reflected)
**Basis:** ADR-0036 (transport relay unification · no phasing · concurrency-critical) · ADR-0035 (layout authority = src-tauri) · ADR-0037 (transport semantics = Rust alone) · TRD rev.5 · **research `docs/research/tauri-channel-multiwindow-carrier-research-2026-06-28.md`**

> A concurrency-critical module. Moves the transport semantics of the current TS `wsTransport.ts` (389 lines) + `protocolClient.ts` (435 lines) to src-tauri Rust. Coding goes in units T1~T8, each coder → `/review code deep` → QA.

---

## 0. Summary
- Create in src-tauri a `DaemonClient` (connection · protocol semantics) + an `OutputRouter` (ViewManager routing, lock-free snapshot). The frontend keeps the existing `ProtocolClient` interface (ADR-0011) on top of `TauriTransport` (a thin carrier).
- **Reused Rust assets:** protocol crate `AgentCommand/AgentEvent/decode_frame/encode_terminal_frame`, discovery crate `read_live_daemon` · the synchronous tungstenite pattern, src-tauri `ViewManager` (the base of the routing table).
- **First obstacle:** src-tauri's Cargo.toml has **none at present** of tokio · tokio-tungstenite · futures-util · serde_json (removed in ADR-0029). Restoring them is T1.
- **Daemon-unchanged confirmation:** the daemon WS server (`ws.rs`) is the same with 1 client or N, manages only ConnId and does not know Views → "daemon unchanged" of ADR-0036 is a fact in the code.

## 1. Concurrency invariants the current TS upholds (to move to Rust)

| TS location | Invariant | Race scenario |
|---|---|---|
| `wsTransport` `openGen` generation token | zombie-socket guard. ++ on every openSocket entry; discard if myGen!==openGen after an await resumes or just before socket creation | start()/close() slips in during an await yield and the resumed run() hijacks this.ws |
| `wsTransport` `pendingReject` | the resolve/reject of a detached socket is trapped in the closure and never called → wake it from outside | if a second start() does not wake the first promise, the caller hangs |
| `wsTransport` `cleanupSocket` #13133 | delete the handlers (not null) then close — prevents an old onclose from clobbering the new socket | an abandoned socket's onclose nulls the new this.ws |
| `wsTransport` `scheduleReconnect` | exponential backoff 500ms→10s MAX 5 → down. attach-only (read_daemon_info no-spawn) | a daemon respawn as a command side effect (B-1) |
| `wsTransport` Auth/Hello | the first frame = Auth, Hello = authentication success (consumed internally) | raising Hello to control would mislead |
| ensureReady vs start (ADR-0021) | ensure = attach-only, start = the only explicit spawn entry point | after the daemon is turned off, keys/resize respawn it |
| `protocolClient` epoch guard | drop an output frame with epoch≠st.epoch | leftovers of an old session pollute the screen |
| `protocolClient` seq high-water dedup | drop seq<=lastDeliveredSeq | duplicate delivery at the reconnect boundary |
| high-water reset on epoch change | when current_epoch changes, lastDeliveredSeq=-1 | prevents wiping out the new session's output |
| `resubscribeAll` | on the connected re-transition, epoch=st.epoch + after_seq=last → Resume (tail-only) | if epoch=null, full replay duplicates |
| request_id pending map | match side-effect commands by randomUUID, echo a dedicated reply variant | lookup responses piggybacking on broadcast |
| connected→disconnect pending reject | reject every in-flight command (once) | promise leak |
| `eventBus` resync guard | getAgents+refreshProfiles only on a connected *re*-transition | first-connection duplicates / missing changes while disconnected |

**Routing:** none currently — each window connects directly to the daemon over its own WS (N in total), subscribing only to its own agentIds. This is the N duplication ADR-0036 aims to remove.

## 2. Rust relocation design

### Placement
```
src-tauri/src/
  daemon_client/         ← 신규 ★동시성-치명
    mod.rs               DaemonClient (연결 핸들 + 명령 API)
    connection.rs        재연결·generation 가드·Auth/Hello (wsTransport 이전)
    protocol_state.rs    epoch·seq dedup·resubscribe·pending (protocolClient 이전)
  output_router.rs       ← 신규 ★lock-free snapshot(arc-swap)
  layout/                ← 모듈②(완료), OutputRouter가 의존
  commands/agent.rs      ← 신규 invoke: spawn/kill/write/resize/subscribe
src/api/tauriTransport.ts ← 신규 Transport 구현(얇은 carrier)
```

### DaemonClient — a single-connection actor
- Runtime tokio multi-thread (tokio-tungstenite 0.26 + futures-util split like the daemon). discovery's synchronous tungstenite is for fire-and-forget so it is unsuitable for a permanent connection → async.
- **A single connection task (actor)** solely owns the `WebSocketStream` · `pending: HashMap<RequestId,oneshot>` · `subs: HashMap<AgentId,SubState>` (no Mutex, isomorphic to protocolClient's self-contained state). invoke does `cmd_tx.send(req{cmd,reply:oneshot})` then awaits.
- The generation guard (openGen) is naturally replaced by **task restart** (the old task's channel closes and it is discarded) — ★but an assumption, needs verification (§6 risk 1).
- State (connected/reconnecting/down) · control events are an `app.emit` broadcast, request replies are oneshot.

### OutputRouter — lock-free (FIX-F6)
```rust
pub struct OutputRouter { table: ArcSwap<RoutingTable> }  // arc-swap: 읽기 lock-free
struct RoutingTable { by_agent: HashMap<AgentId, Vec<WindowSink>> }
```
- Read (hot path): `table.load()` → by_agent.get → deliver to each WindowSink. Zero locks.
- Write (low frequency): after a ViewManager mutation (after the lock is dropped, in the emit stage) recompute the table → `table.store`. No ADR-0006 violation.

### TauriTransport (frontend)
- Implements the `Transport` interface; the brain is Rust. `send`→invoke('agent_command'), `onMessage`→listen + output Channel, state→listen('daemon-connection-state'), start/ensure/close→invoke. `clientFactory` goes from `new WsTransport()`→`new TauriTransport()`. agentClient = the ProtocolClient singleton kept (ADR-0011). The ensure/start split moves to Rust.

## 3. Invariant mapping (Rust preservation + race risk)

| Invariant | Rust preservation | Race risk |
|---|---|---|
| zombie guard (openGen) | task restart (old task's channel closed) + explicit generation | ★high — if the "resolved naturally" assumption is wrong this is the riskiest. Precisely test the tokio abort timing and the resumption of in-flight awaits. Both cases of TS `[Blocker-1]` must be ported |
| reconnect backoff | tokio sleep loop inside the task + read_live_daemon (no-spawn) MAX5→down | medium |
| ensure/start split | DaemonClient connect()/ensure() split | medium — with a single client, simultaneous ensure from multiple WebViews disappears (a gain), and discovery's ensure_lock may become unnecessary (D5) |
| epoch guard | connection task SubState | low (sequential) |
| seq dedup | SubState last_delivered_seq, replay_from is not used as a criterion (bug-B guard) | low |
| high-water reset on epoch change | -1 when current_epoch changes | low |
| resubscribe resume (bug A) | on the connected re-transition iterate subs with Subscribe{epoch:Some,after_seq:last} | medium |
| replay→live order | **daemon-owned (ws.rs single-writer FIFO)** — the client only does seq dedup | low |
| finalize once (ADR-0005) | **daemon-owned — irrelevant to the client** | none |
| kill causality (ADR-0001) | **daemon-owned — the client sends Kill + Ack** | none |
| lock order (ADR-0006) | OutputRouter hot path has 0 locks (arc-swap), the table update happens after the ViewManager lock is dropped | medium — putting the update inside the lock violates it |
| pending matching | connection task HashMap | low |
| pending reject on disconnect | on task end/restart all Err | medium — missing cleanup of old pending on task replacement |

**Three core race spots:** ① in-flight commands/subscriptions during task replacement (Blocker-1 port assertions) ② OutputRouter update ↔ ViewManager lock order ③ reconnect resubscribe with a concurrent unsub/re-subscribe.

## 4. Coder unit breakdown (T1~T8)
```
T1 deps복원 → T2 connect/handshake → T3 protocol_state(epoch/dedup/pending)
                    │                        │
                    │                  T4 reconnect+resubscribe(★Blocker-1)
                    ▼                        │
              T5 OutputRouter(arc-swap)◄─layout(②)┘
                    │
              T6 commands/agent.rs invoke
                    │
              T7 TauriTransport+clientFactory 교체
                    │
              T8 React 정리(slotStore삭제·PopupPage·eventBus)+InProc mock(D4)
```

| T | Content | Isolated harness (without a real daemon) |
|---|---|---|
| T1 | Restore tokio · tokio-tungstenite · futures-util · serde_json · arc-swap in Cargo.toml + the AppState manage slot | cargo build |
| T2 | DaemonClient connection task: discover→WS→Auth→Hello, connect/ensure split | integration bin (reuse the daemon's `tests/ws_e2e.rs` start_test_server) or a mock WS server |
| T3 | protocol_state: epoch · dedup · resubscribe · pending **split into pure functions** | headless unit — port the 21 cases of `protocolClient.test.ts` 1:1, inject SubState directly |
| T4 | Reconnect backoff + generation guard + read_live_daemon | headless unit + tokio `time::pause()` — port Blocker-1 · hot-swap · exhaustion→down of `wsTransport.test.ts` |
| T5 | OutputRouter arc-swap + update trigger | headless unit — verify ViewManager mutation→table recompute→load, assert 0 locks on the hot path |
| T6 | invoke (spawn/kill/write/resize/subscribe)→DaemonClient | integration bin |
| T7 | TauriTransport + clientFactory replacement | vitest invoke mock |
| T8 | React cleanup + InProc mock handling (D4) | vitest + cargo test root regression + cdp.mjs measurement |

**The 2 TS test files (40+ cases) are the specification for the Rust port.** The isolation foundation already exists: discovery Clock/DaemonReader trait injection, the daemon's start_test_server.

## 5. User decisions needed (D1~D5)

- **D1 (biggest) location of the dedup/epoch guard:** (A) Rust alone — kill ProtocolClient's handleOutput and Rust emits only clean chunks (single source of truth · once before routing, but ProtocolClient becomes a shell → conflicts with the ADR-0011 identity) / (B) Rust primary + JS defensive secondary (existing kept · gradual, semantics in two places) / (C) JS alone · Rust raw relay (needs a partial decode of agentId for routing + the N-window guard N times → halves the ADR-0036 benefit).
- **D2 ProtocolClient thickness:** dependent on D1. With A it nearly vanishes and TauriTransport is the client / with B·C it stays nearly as is + only the carrier.
- **D3 routed output carrier:** Tauri Channel (binary efficiency, complex coupling to the window lifecycle) vs emit_to (simple, JSON bloat). Directly tied to high-bandwidth output → §6 performance.
- **D4 InProc mock tests:** the `protocolClient.test.ts` InProc describe + MockTransport (traces of ADR-0020). Discard / port to Rust / rewrite for TauriTransport.
- **D5 `commands/discovery.rs` ensure_lock:** with a single DaemonClient, simultaneous ensure from multiple WebViews disappears so it may be unnecessary. Remove vs keep.

## 6. Risks · unknowns
1. **Blocking zombie/hijack on connection task replacement (T4)** — that the task model "structurally resolves" the openGen guard is an assumption. A new race is possible in the tokio abort timing and the oneshot drop order. Passing the Blocker-1 port is the safety gate.
2. **OutputRouter update ↔ ViewManager lock order (ADR-0006)** — the arc-swap hot path is OK, but putting the update trigger inside the lock violates it + deadlocks.
3. **The union of N-window subscriptions on reconnect resubscribe** — subscribing once per agent (deduplicated) needs a "subscription ref-count" (Unsubscribe only when the last window closes). A new concurrency surface that TS did not have.
- **Measuring high-bandwidth relay overhead:** daemon→src-tauri WS (once)→Router→window (IPC) adds one hop. With D3 emit_to there is a risk of reviving JSON/base64 bloat. Measure throughput with `cdp.mjs eval` (ADR-0036's "local IPC is negligible" is an assumption, verified in QA full).
- **Unknowns (code not checked):** per-window creation/cleanup safety of Tauri 2 `ipc::Channel` in multi-window (check with context7) · the interaction between reintroducing tokio multi-thread in src-tauri and the Tauri runtime.

## 7. Research reflected (2026-06-28) — T5~T8 confirmed details

> The cross-family deep research (report: `docs/research/tauri-channel-multiwindow-carrier-research-2026-06-28.md`) closed the unknowns (§6) of the D3 carrier and the fan-out design. **Per current version: `tauri = 2.11.2` / `@tauri-apps/api ^2.11.0`.**

### D3 carrier — Channel confirmed + ★raw byte pitfall
- Carrier split confirmed: **high-bandwidth output = `Channel` (per-window) / layout control = `emit`** (low-frequency JSON is OK, module ② current state kept). The official docs explicitly name "child process output" as a Channel use case.
- **★ Send terminal bytes with `Channel<tauri::ipc::Response>` (`Response::new(bytes)`) or `InvokeResponseBody::Raw`.** `Channel<Vec<u8>>`/`Channel<&[u8]>` are **serialized as a JSON array** by the blanket `impl<T:Serialize> IpcResponse` (even the official 4096B example's `Channel<&[u8]>` leaks as JSON). Carrying the bytes of the protocol crate's `encode_terminal_frame` in Response/Raw is consistent.
- A Channel is natively bound to the calling webview → a Channel per window solves per-window routing naturally (emit_to unnecessary).

### T5 OutputRouter — arc-swap snapshot (open item resolved)
- `ArcSwap<RoutingSnapshot { by_agent: HashMap<AgentId, Arc<[WindowId]>> }>` — **the hot path (every frame) `load()` takes 0 locks**, `store(Arc::new(..))` only on a layout change. The arc-swap official "routing table read with every packet" use case.
- **Pitfall (assert with tests):** do not hold the `load()` Guard across an `.await` (slot exhaustion) → use `load_full()` when going through async. (ADR-0006 hot path 0 locks = already §2 FIX-F6.)
- Rejected alternatives: `RwLock` (read contention), `broadcast` (a slow receiver's `Lagged` **loses data** → dangerous for a lossless terminal), `left-right`/`evmap` (eventual-consistency complexity), `watch` (OK for snapshot distribution · unsuitable for hot-path lookups).

### Subscription union ref-count (resolves §6 risk 3 "new concurrency surface")
- **Separated** from the hot path: `Mutex<HashMap<AgentId, usize>>` (changed only on layout/window lifetime events). **0→1 transition = `Subscribe` / 1→0 = `Unsubscribe`.**
- Bind it to the slot/window lifetime with `SubscriptionGuard { agent_id, owner }` + `Drop`. **★ No async Drop** — `Drop` cannot await → **enqueue** the unsubscribe on the **DaemonClient cmd_tx** (consistent with the actor model, no direct await).

### Cleanup · lifecycle conventions (T6/T7)
- **dead window:** `Channel::send` returns `Result<()>` — `Err` for a destroyed webview (the error type is undocumented). **On detecting `Err`, remove that channel from the routing registry** (never `unwrap()`).
- **window close:** explicit `unsubscribe` + `unlisten` in `onCloseRequested` — **#15583 (backend listeners not cleaned up when a webview is destroyed) is unresolved in 2.11.2**.
- **Large queue:** `ChannelDataIpcQueue` may linger (the window closes before fetching) → confirm cleanup on close/unsubscribe.
- **Frontend channel cleanup:** `delete channel.onmessage` (not null — #13133, existing micro-rule kept).

### ⚠️ Version (dependency — a decision-report item)
- **#13133 (callback leak, fixed in v2.5.0) · #12065 (order inversion, fixed in 2.2.0) = already resolved in our 2.11.2.**
- **The channel-data deadlock is fixed in tauri 2.11.3 — we are on 2.11.2 (one patch below).** Consider an **upgrade to 2.11.3+** before wiring the T5~T7 Channel (a dependency change = report to the user).

### Unverified (measurement area — QA full)
- The raw Channel's actual throughput · one-hop overhead is estimated from docs/source → confirm ADR-0036's "local IPC is negligible" assumption with a `cdp.mjs eval` throughput measurement (acceptance criterion §3 trd / spike §6).

## 8. T5 OutputRouter — TRD details (2026-06-29)

> Integration surface mapping (Explore) + carrier research (§7) reflected. **Purely internal decisions are confirmed (below); the two major forks (F-A/F-B) await user decisions.**

### Confirmed seams · types (measured in code)
- **Input seam:** `src-tauri/src/daemon_client/connection.rs:668` the `Message::Binary(bytes)` spot (currently `// TODO(T5)`). Flow: `protocol::decode_frame(bytes) → DecodedFrame{agent_id, epoch, seq, payload:&[u8]}` → `protocol_state::decide_output(&mut sub, epoch, seq)` → **if `Deliver`, `router.route(agent_id, payload, seq)`**, ignore `Drop*`. (Inject `router: &OutputRouter` Arc into `main_loop` — shared app-level across reconnects.)
- **Routing source:** `layout/manager.rs` `ViewManager{views, active_view_id, window_bindings:HashMap<label,view_id>, version}` + `LayoutNode::Slot{agent_id:Option<String>}`. `agent_id → [window_label]` = search each View tree for that agent's slots → (if View==active_view_id then "main") + (the labels bound to that view_id in window_bindings).
- **Output carrier:** **one per-window `Channel<tauri::ipc::Response>`** carries all agent output of that window (frames tagged with agent_id). The window registers through the T6 invoke (`subscribe_output(channel)`). (§7 raw byte: load as `Response`/`Raw`.)
- **★Type bridge (coder beware):** the frame's `agent_id` = `AgentId` (protocol, UUID newtype) ↔ the slot's `agent_id` = `String`. Normalize the routing key to one side (recommended: `AgentId` → the same string at slot-store time). Reuse the existing String↔AgentId conversion of the spawn path.

### Internal decisions (confirmed — for reporting, not user decisions)
- **D1 WindowId = Tauri window label (String)** — same as the `window_bindings` key, no separate numeric registry needed. `RoutingSnapshot{ by_agent: HashMap<AgentId, Arc<[String]>> }`.
- **D2 rebuild-always** — layout changes are low-frequency, so recompute the whole snapshot on every change then `ArcSwap::store`. A version-cache branch is not adopted (no gain for the complexity).
- **D3 the rebuild call = `router.rebuild(&mgr)` made *while holding* the ViewManager lock (in the same critical section as the layout mutation)** → the returned delta is sent by T6 over cmd_tx **after unlock**. **(★rev 2026-06-29 — `/review code deep` 3-reviewer convergence: the old "emit_after_unlock (call outside the lock)" makes `load(prev)→delta→store` non-atomic, so concurrent rebuilds cause delta drift [duplicate Subscribe · missing Unsubscribe] + ABA [a stale store overwrites a new store]. The existing ViewManager lock serializes the RMW + guarantees the currency of `&mgr`.)** Not an ADR-0006 violation — the body = pure computation + lock-free `ArcSwap::store`, 0 external I/O inside the lock (0 emit/DaemonClient/network), only sending after unlock. A separate rebuild mutex · callback/back-reference not adopted (reusing the existing lock is enough).
- **D4 carrier = per-window Channel (tagged)** — §7 research confirmed (a per-(window,agent) channel explosion rejected / emit broadcast rejected for JSON · correctness).
- **D6 cleanup** — `Channel::send` `Err` → remove that window sink + rebuild (never unwrap) · on window close `onCloseRequested` → explicit unsubscribe + unlisten (#15583 unresolved in 2.11.3).

### Major forks — decided (2026-06-29, user)
- **F-A slicing = ① T5 alone first.** Only the OutputRouter core (snapshot recompute + route), verified in isolation with headless unit (ViewManager mutation→table→load · assert hot-path 0 locks) → afterwards T6 (invoke + channel registration) · T7 · T8 each coder→`/review code deep`→`/qa`.
- **F-B subscription union source = ① layout-derived.** No separate counter — on every snapshot rebuild, diff the current agent union set against the previous one for 0→1 `Subscribe`/1→0 `Unsubscribe`. **Piggyback on the same tree traversal as the routing snapshot recompute (single pass: routing table + subscription diff produced together).** Rationale: SSOT = ViewManager (ADR-0035), the only output consumer is the View + the daemon keeps the output (re-subscribing after Unsubscribe = a `Resume(after_seq)` tail replay so 0 loss), so ② (explicit ref-count) has no benefit for now. **No direct await in async Drop (even if a cleanup path is used, enqueue on cmd_tx).**
  - **Extension memo (YAGNI — not now):** if a non-screen output consumer appears (e.g. the §5 backend LLM consuming another agent's output programmatically = goal ⑤ messaging track, a1 · deferred), extend to "the layout-derived set of ① ∪ a separate subscriber set". Do not adopt ② wholesale.
  - **Concept separation (to avoid confusion):** output subscription (T5, render target) ≠ focus (`focused_slot_id`, key-input target) ≠ agent↔agent messaging (goal ⑤, data-plane). T5 is output subscription only.

## 9. T6 — wiring TRD (2026-06-29, integration surface mapping reflected)

> **Real IPC wiring** on top of T5 (the routing core, headless). A 7+ file integration — needs its own `/review code deep` round. Most forks are internal decisions (for reporting), confirmed below. **Adjacent to concurrency-critical** (connection task ↔ Tauri Channel ↔ layout lock).

### T6 sequence (coder units)
1. **Fill the `ConnectionCommand` variants** (`connection.rs:~131`, currently `__Placeholder`): `SendCommand{cmd: AgentCommand, reply: oneshot}` + `Subscribe{agent_id, epoch, after_seq, reply}` + `Unsubscribe{agent_id, reply}`. (Fork B = B1)
2. **`DaemonClient::send_command`** (`mod.rs:~357`, currently TODO): expose a wrapper that awaits a oneshot after `cmd_tx.send`. **On disconnect drain the pending oneshots** (consistent with T4 reconnect cleanup — needs confirmation).
3. **AppState registration** (`lib.rs` setup): `Arc<DaemonClient>` · `Arc<OutputRouter>` · **window Channel registry** `Arc<Mutex<HashMap<WindowLabel, Channel<Response>>>>`. (Fork A = A1)
4. **`commands/agent.rs` (new)**: invoke spawn/kill/write/resize → build `ConnectionCommand::SendCommand` + await reply. + the **`subscribe_output(label, channel)`** invoke = registry insert on window mount. (Fork D/E = D1/E1)
5. **Modify all mutation commands of `commands/layout.rs`**: call `router.rebuild(&mgr)` **inside the lock** in the critical section (FIX-1/D3), and **after unlock** delta→`send_command(Subscribe/Unsubscribe)`. (Fork C = C1 confirmed)
6. **`connection.rs` main_loop wiring**: (a) cmd_rx arm (`:686`) — wire-encode the variant and `sink.send` + reply handling. (b) Binary arm (`:668`) — `decode_frame → decide_output → Deliver면 router.targets(agent_id) → registry 의 각 창 Channel 로 fan-out` (`Response::new(bytes)` raw, §7).

### Confirmed internal decisions (forks — for reporting)
- **A=A1** window Channel registry = `Arc<Mutex<HashMap<WindowLabel, Channel<Response>>>>` in AppState. The connection task holds an Arc clone, lock→send when routing. Window close → deregister + send `Err` → registry remove (dead window). (per-(win,agent) explosion · emit JSON rejected)
- **B=B1** ConnectionCommand 3 variants + oneshot reply. (Box<dyn Any> type erasure rejected)
- **C=C1** rebuild inside the lock / delta sent after unlock (FIX-1 · D3 confirmed).
- **D/E=D1/E1** one Channel per window registered via `subscribe_output` on mount, on close deregister + #15583 unlisten.

### ★T6 unresolved — to be resolved before the coder enters
- **(G1) Source of Subscribe epoch/after_seq:** the delta only knows "subscribe to agent X". epoch/after_seq must come from the **protocol_state `SubState`** (owned by the connection task, T3) — new = epoch None (full replay) / re-subscribe = tail-only (reuse `resubscribe_params`). A decision is needed on whether to fill the delta→Subscribe conversion **inside the connection task** by looking up SubState, or in the layout command (the former is consistent — SubState is owned by the task).
- **(G2) `Channel::send` from a tokio task:** measure whether it is safe for the connection task (tokio runtime) to call Tauri's `Channel::send` (Channel is Send+Sync, internal webview.eval marshaling — likely but unverified). Measuring first-output arrival with `cdp.mjs` = the T6 GUI gate.
- **(G3) registry ↔ connection task lifetime:** the registry is owned by AppState (Tauri) and the connection task by DaemonClient — how to inject the registry Arc into the task (add an argument to start_connection). Confirm the layout label ("main"/"slot-popup") ↔ window_bindings label match.

## 10. T6b done + remainder (deferred to T7) (2026-06-29)

**Done:** output-plane wiring — Binary arm fan-out (`decode_frame`→`decide_output` guard→`router.targets`→window `Channel<Response>`) · SubscribeAck epoch update (`apply_subscribe_ack`) · Subscribe/Unsubscribe/Fire (fire-and-forget) · **resubscribe based on `router.current_agents()`** (C1+C2) · **`mark_delivered` separated** (the high-water advances only after fan_out succeeds in 1+ windows, C4) · rebuild+delta on layout's 6 mutations (rebuild inside the lock / send outside the lock) · the `subscribe_output` invoke · window Channel registry (`output_channel.rs`). `cargo test --workspace` **148 green**.

**Verification:** `/review code deep` 2 rounds (opus doc-aware ×2 + Codex blind). Round 1 raised C1~C4 → fixed → round 2 re-verification = **C1~C4 closed** (data corruption/contamination 0). ADR-0006 (0 awaits while holding the fan_out lock · rebuild inside the lock / send outside the lock) · ADR-0037 (guard once before routing) fine.

**★Remainder = T7 area** (currently a **dead path** — the frontend still connects directly through `wsTransport` and does not use this path, so the screen does not break):
- **N1** multi-window / late mount: when the same agent is shown in several windows at once, a per-agent single seq cannot track per-window progress + there is no trigger to re-deliver undelivered frames.
  - **Root cause (research 2026-06-29, `research/study-notes/`):** we treat output as a **seq-queue consumption model** (per-agent high-water + dedup), but the terminal industry standard is the **state-render model** (tmux/Zellij/VS Code — the server keeps it centrally + redraws on attach, no single consumed cursor on the output). → **Separating seq (lossless transport between daemon↔gateway) from render (filling the window)** is the direction to resolve N1.
  - **Fork on filling a new window (decided in the T7 PRD/TRD):** (A) re-request from the daemon (industry standard · simple · 2-hop loopback) vs (B) a src-tauri gateway cache (fits 2 layers · non-standard · sync cost). Research conclusion: central storage + re-request is the standard; a gateway cache is only a convenience for the web renderer, not an authoritative store.
- **N5** operating-time cleanup of subs (retain happens only at reconnect — a leak remains while the connection stays up). **N6** duplicate Subscribe when a layout mutation happens concurrently during resubscribe (confirm the idempotency of the daemon's Subscribe). **N4** no integration test for the fan_out Binary wiring (headless is possible once a seam is introduced / or T7 GUI measurement).

**T7 = replacing `TauriTransport` + `clientFactory`**: decide the transport/render separation · A/B above via a PRD/TRD + GUI measurement (G2 = real window output arrival, `cdp.mjs`).

## 11. T7 TRD — TauriTransport cutover + N1 (seq/render separation) (2026-06-29)

> T7 goal: actually activate the T6b wiring (replace clientFactory) while also closing N1 (multi-window · late mount), which was postponed while it was a dead path.

### Confirmed decisions

- **N1 direction = plan B (no daemon protocol change):** the daemon knows only agents and does not know windows. The per-window render cursor is managed only inside src-tauri. Plan A, which passes a window_id to the daemon, is rejected as a layer violation.
- **epoch stays per-agent:** it is an agent-session concept, so it is unrelated to windows. `SubState.epoch` is kept.
- **render_seq is per-window per-agent:** newly placed in `WindowEntry.render_seqs`.

### Problem of the current structure (root of N1)

`SubState.last_delivered_seq` mixes two roles:

```
역할 ①  재연결 after_seq 계산 (데몬↔src-tauri 전송 레벨)
역할 ②  이 프레임이 창에 도달했는지 추적 (렌더 레벨)

→ per-agent 단일 카운터라, 새 창이 Subscribe(after_seq=None) 요청을 보내도
  기존 decide_output이 seq ≤ last_delivered_seq → DropDuplicate 처리해 버림.
```

### Whole map of the structural change

```
변경 전                                변경 후
─────────────────────────────          ──────────────────────────────────────────
SubState                               SubState
  epoch: Option<u32>            ───▶     epoch: Option<u32>   ← 그대로
  last_delivered_seq: Option<u64>        (seq 필드 제거)

WindowChannelRegistry                  WindowChannelRegistry
  HashMap<WindowLabel, Channel>  ───▶   HashMap<WindowLabel, WindowEntry>
                                          WindowEntry {
                                            channel: Channel<Response>,
                                            render_seqs: HashMap<AgentId, Option<u64>>,
                                          }

decide_output(st, epoch, seq)          decide_epoch(st, epoch) → EpochDecision
  → epoch check + seq dedup    ───▶     → epoch check만 (seq 판단 제거)

mark_delivered(st, seq)         ───▶   (제거 → 창별 render_seq 갱신으로 대체)

fan_out: 전 창 전송 → mark             fan_out: epoch check → 창별 seq check
                                          → send + render_seq 갱신

resubscribe_params(st)          ───▶   after_seq = registry.min_render_seq(agent_id)
  after_seq = st.last_delivered_seq       (모든 창 중 최솟값, 어느 창이라도 None → None)
```

### Per-file change specification

#### `protocol_state.rs`

```rust
// SubState: last_delivered_seq 제거
pub struct SubState {
    pub epoch: Option<u32>,
    // last_delivered_seq 삭제
}

// decide_output → decide_epoch (seq 판단 없음)
pub enum EpochDecision { Pass, DropEpochMismatch }
pub fn decide_epoch(st: &SubState, frame_epoch: u32) -> EpochDecision

// mark_delivered 삭제

// resubscribe_params: after_seq를 호출자가 외부 주입
pub fn resubscribe_params(st: &SubState, after_seq: Option<u64>) -> SubscribeParams
//   after_seq = registry.min_render_seq(agent_id)로 호출자가 계산해 넘김
```

- The existing seq dedup tests (dedup_same_seq_dropped etc.) → move to the `output_channel.rs` tests
- The epoch-only tests are kept as is, with only the signatures adjusted

#### `output_channel.rs`

```rust
pub struct WindowEntry {
    pub channel: Channel<tauri::ipc::Response>,
    pub render_seqs: HashMap<AgentId, Option<u64>>,  // None = 아직 아무것도 렌더 안 함
}

pub type WindowChannelRegistry = Arc<Mutex<HashMap<WindowLabel, WindowEntry>>>;

// 헬퍼 (Mutex 보유 중 호출 — 모두 순수 동기)
impl WindowEntry {
    /// seq가 last보다 크면 true (deliver 대상). None(미렌더) → 항상 true.
    pub fn should_deliver(&self, agent_id: AgentId, seq: u64) -> bool

    /// 렌더 성공 후 render_seq 전진.
    pub fn mark_rendered(&mut self, agent_id: AgentId, seq: u64)

    /// epoch 변경 시 이 창의 agent_id render_seq 리셋(None).
    pub fn reset_agent_seq(&mut self, agent_id: AgentId)
}

// registry 수준 헬퍼
/// 모든 창의 render_seq 중 최솟값. 어느 창이라도 None → None(처음부터).
/// 이 agent를 보는 창이 없으면 after_seq=None(= 어차피 Subscribe 안 보내도 됨, 호출자가 판단).
pub fn min_render_seq(registry: &HashMap<WindowLabel, WindowEntry>, agent_id: AgentId) -> Option<u64>

/// epoch 변경 시 모든 창의 agent render_seq 리셋.
pub fn reset_all_windows_for_agent(registry: &mut HashMap<WindowLabel, WindowEntry>, agent_id: AgentId)
```

Unit tests:
- `should_deliver`: None → true, Some(last) → seq>last=true / seq≤last=false
- `mark_rendered`: monotonic advance
- `min_render_seq`: mixed (Some/None) → None, all Some → the min value, no windows → None
- `reset_all_windows_for_agent`: that agent's seq in all windows = None

#### `connection.rs` Binary arm (fan_out part)

```rust
// 변경 전
Message::Binary(bytes) → decode_frame → decide_output(sub_state) → Deliver면 fan_out → mark_delivered

// 변경 후
Message::Binary(bytes) → decode_frame → decide_epoch(sub_state, frame_epoch)
  → DropEpochMismatch: 폐기
  → Pass:
      let targets = router.targets(agent_id);   // lock-free
      if targets.is_empty() { continue; }
      let mut registry = registry.lock();
      for window_label in targets.iter() {
          let Some(entry) = registry.get_mut(window_label) else { continue; };
          if entry.should_deliver(agent_id, frame_seq) {
              // Channel::send는 동기 — lock 보유 중 await 없음 (ADR-0006 준수)
              let _ = entry.channel.send(Response::new(frame_payload));  // Err → dead window
              entry.mark_rendered(agent_id, frame_seq);
          }
      }
      // dead window 정리: send Err인 창 registry에서 제거
      // (별도 collect 후 remove — lock 보유 중 정리, send가 동기라 가능)
```

★Concurrency notes★:
- While holding `registry.lock()`, `Channel::send` (synchronous) → no await → ADR-0006 not violated
- `router.targets()` is lock-free, so it is called before the registry lock → no lock order
- dead window cleanup: remove under the same lock after collecting the `send` Errs (no reentrancy)

#### `connection.rs` resubscribe_all

```rust
// 변경 전
fn resubscribe_all(subs, router, cmd_tx) {
    for agent_id in router.current_agents() {
        let params = resubscribe_params(&subs[agent_id]);  // last_delivered_seq 사용
        cmd_tx.send(Subscribe { agent_id, ...params });
    }
}

// 변경 후
fn resubscribe_all(subs, router, registry, cmd_tx) {
    let reg = registry.lock();
    for agent_id in router.current_agents() {
        let after_seq = min_render_seq(&reg, agent_id);   // 모든 창 최솟값
        let params = resubscribe_params(&subs[agent_id], after_seq);
        // reg lock 해제 전 cmd_tx.send — cmd_tx는 mpsc(비동기 가능성), 락 안에서 send 지양
        // → agent_id + params를 collect 후 lock 해제 → send
    }
    drop(reg);
    for (agent_id, params) in collected { cmd_tx.send(...); }
}
```

★ADR-0006★: no cmd_tx.send (tokio mpsc) while holding the registry lock — collect, release the lock, then send.

#### `connection.rs` SubscribeAck handling — reset render_seq on epoch change

```rust
AgentEvent::SubscribeAck { agent_id, current_epoch, .. } => {
    let prev_epoch = subs.get(&agent_id).and_then(|s| s.epoch);
    protocol_state::apply_subscribe_ack(subs.entry(agent_id).or_default(), current_epoch);
    // epoch 변경이면 모든 창의 이 agent render_seq 리셋 (새 세션 = 처음부터)
    if prev_epoch.is_some_and(|p| p != current_epoch) {
        let mut reg = registry.lock();
        reset_all_windows_for_agent(&mut reg, agent_id);
    }
}
```

#### `commands/agent.rs` — subscribe_output change

```rust
// 변경 전: channel만 registry에 insert
// 변경 후
#[tauri::command]
pub fn subscribe_output(
    window_label: String,
    channel: Channel<Response>,
    state: State<AppState>,
) {
    let entry = WindowEntry { channel, render_seqs: HashMap::new() };
    state.registry.lock().insert(window_label.clone(), entry);

    // 현재 이 창이 보여야 할 agent들에 대해 Subscribe 재발송
    // (OutputRouter가 이미 알고 있음 — router.current_agents()는 전체이므로
    //  실제로는 layout에서 이 창에 배정된 agent만. T7에서 단순화: 전체 resubscribe)
    // → 연결 task에 Resubscribe 신호 전송 (cmd_tx로 새 커맨드 variant 추가)
    state.daemon_client.request_resubscribe();
}
```

`request_resubscribe()`: sends `ConnectionCommand::Resubscribe` over cmd_tx → the connection task runs `resubscribe_all`.
resubscribe_all fires right after a new window mounts → min_render_seq is None (new window), so after_seq=None → the daemon does a full replay.

#### `src/api/tauriTransport.ts` (new)

```ts
import { invoke } from '@tauri-apps/api/core'
import { Channel } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

export class TauriTransport implements Transport {
  private channel: Channel<ArrayBuffer> | null = null
  private unlisten: (() => void) | null = null

  async start(): Promise<void> { await invoke('daemon_start') }
  async ensure(): Promise<void> { await invoke('daemon_ensure') }
  async close(): Promise<void> { await invoke('daemon_close') }

  async connect(onMessage: (bytes: Uint8Array) => void, onStateChange: (s: ConnectionState) => void) {
    this.channel = new Channel<ArrayBuffer>()
    this.channel.onmessage = (raw) => onMessage(new Uint8Array(raw))  // raw = Response bytes
    await invoke('subscribe_output', { channel: this.channel })

    this.unlisten = await listen<ConnectionState>('daemon-connection-state', (e) => {
      onStateChange(e.payload)
    })
  }

  async send(cmd: AgentCommand): Promise<AgentEvent> {
    return invoke<AgentEvent>('agent_command', { cmd })
  }

  disconnect() {
    delete this.channel?.onmessage  // #13133 — null 아님
    this.channel = null
    this.unlisten?.()
    this.unlisten = null
  }
}
```

#### `src/api/clientFactory.ts`

```ts
// 변경 전
export const agentClient = new ProtocolClient(new WsTransport(...))

// 변경 후
export const agentClient = new ProtocolClient(new TauriTransport())
```

### Implementation breakdown (T7a~T7d)

| Step | Content | Isolated harness |
|---|---|---|
| T7a | `protocol_state.rs` SubState shrink + `decide_epoch` + test fixes | `cargo test -p engram-dashboard-core` |
| T7b | `WindowEntry` extension + `output_channel.rs` helpers + `connection.rs` fan_out · resubscribe · epoch reset | headless unit (output_channel) + cargo test --workspace |
| T7c | `TauriTransport.ts` + `clientFactory` replacement | vitest mock invoke |
| T7d | GUI measurement (G2) + N5/N6 handling | cdp.mjs + `/qa full` |

Each step: coder→`/review code deep`→`/qa`.

### Verification criteria

- **G2 (T7d):** `cdp.mjs eval` → confirm agent output reaches a real window
- **Multi-window (T7d):** the same agent in two windows → output reaches both windows, no duplicates in the existing window
- **Late mount (T7d):** close a window and reopen it → receives the replay of existing output
- **Reconnect (T7b):** mock WS disconnect → no undelivered output after reconnect
- **cargo test --workspace:** stay fully green after T7b

### N5/N6 handling (T7d bundle)

- **N5** operating-time cleanup of subs: after `resubscribe_all`, `retain` away the agent entries that are not in the router from `subs` (what used to be done only at reconnect → run on every resubscribe_all).
- **N6** duplicate Subscribe idempotency: close N6 after confirming the idempotency of the daemon `ws.rs` Subscribe (needs a code check, before entering T7d).

---

## Reference code
`src/api/wsTransport.ts` (the original being moved) · `protocolClient.ts` (the original being moved) · `crates/engram-dashboard-daemon/src/ws.rs` (server actor symmetry) · `src-tauri/src/layout/manager.rs` (routing source) · `src/api/wsTransport.test.ts` · `protocolClient.test.ts` (Rust port specification) · `crates/engram-dashboard-protocol` (`encode_terminal_frame` → loaded into Response/Raw).
