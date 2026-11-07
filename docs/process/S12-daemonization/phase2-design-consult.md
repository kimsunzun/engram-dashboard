# Phase 2 daemon implementation design — consult cross-validation merge (2026-06-14)

Basis: `/consult` with 3 models (GPT, Gemini, Claude-opus), blind + judge. Raw material: `agents/web-runner/shared/20260614-151528-consult-daemon-phase2/`. Merged with direct code verification. The judge misattributed a GPT contribution to Gemini → corrected by the manager (what follows reflects the correction).

## Contributions and errors per model (un-blind)
- **GPT (most trustworthy, 0 errors):** ★the missing ReplayBuffer **event-count cap** risk (1-byte-chunk runaway → replay events explode → a new subscriber's mpsc is immediately full → permanent slow-consumer disconnect on every reconnect) + ★the missing **WsStatusSink** (status/list/restore_result also need WS text frames, otherwise the agent list cannot be updated → termination cannot be decided). Both are real in the code.
- **Claude (0 errors, co-best):** separation of the WMI **2-layer structure** (upper: Tauri→daemon WMI once / lower: daemon→PTY portable-pty N times, no breakaway needed) + the **ineffectiveness of ACL for the same SID** (a process of the same user cannot be blocked by ACL; the real defense = blocking token exposure on the command line) + correctly recognizing that replay stores raw.
- **Gemini (2 errors — discarded):** ① the "Control Lane deadlock" in the split structure is not real (read/write are independent tasks, so write backpressure does not block read) ② misapplied the WMI race and CREATE_SUSPENDED to the PTY child spawn (the PTY child is spawned directly by portable-pty, so WMI is irrelevant) ③ the misconception that ReplayBuffer stores base64 (it stores raw in the code). Unique contributions: Stale Port Hijacking / Graceful Shutdown / Malicious Flood harness cases (valid).

## 3-model agreement (confirmed by the judge, adopted)
1. **"Zero core changes" = false.** The current emit encodes raw→base64 and fans out only as PtyEvent (the sink receives only base64; raw goes only to ReplayBuffer). WS needs raw → a decode·re-encode round trip. **A switch to raw-first is mandatory.**
2. **Calling try_send from a std thread (synchronous) is safe** — non-async, non-blocking. What is forbidden is send().await/blocking_send.
3. **core has no dependency on protocol; the daemon translates.** Rename to SpawnSpec (core profile::AgentCommand→SpawnSpec, serde variants kept for disk compatibility).
4. **A single writer queue per connection (conn_tx).** SplitSink cannot be written concurrently. Output frames and control JSON all join the single conn_tx.
5. **On connection close, unsubscribe all of that connection's (agentId, SinkId) at once** (prevents leaks) — per-connection registry.
6. **The SubscribeAck→replay→live ordering race is real** — serialized by the enqueue order of the single conn_tx.

## Merged implementation design (ready to start)

### Step 1 — Core raw boundary (★top priority, verifiable in code)
- `core/pty/types.rs`: change the signature to `OutputSink::send(&self, chunk: &OutputChunk)` (current `PtyEvent` → raw `OutputChunk{seq, data:Vec<u8>}`).
- `core/pty/output_core.rs`: remove the base64 encoding from emit; both the ReplayBuffer push and the fanout use raw `&OutputChunk`. The subscribe replay is the same.
- `src-tauri/lib.rs` ChannelOutputSink: inside send, base64::encode → PtyEvent → Tauri Channel (preserves Embedded behavior). **Moving the base64 responsibility to the sink** = the core becomes transport-agnostic.
- WsOutputSink (daemon, phase2 Step4): raw → `protocol::encode_terminal_frame` (0 decode round trips).
- Verification: core unit 38 + GUI E2E (Embedded base64 decode regression, same method as phase1b).
- ★Note: PtyEvent (base64) is isolated inside ChannelOutputSink as an Embedded-only type. Unrelated to the protocol wire.

### Step 2 — Rename to SpawnSpec
Core `profile::AgentCommand` (Claude/Shell) → `SpawnSpec`. All references updated (profile/backend/manager). Keep the serde variant representation (agents.json compatibility). The protocol `AgentCommand` (envelope) is unchanged.

### Step 3 — Protocol addendum
- **Add an Auth first frame** (not in the current protocol AgentCommand — omission). `ClientHello{token, protocol_version}` or a separate auth frame.
- **Add a ReplayBuffer max_events cap** (GPT): `max_events` (e.g. 4096) together with `max_bytes` (2MB). Evict condition `total_bytes>max_bytes || events.len()>max_events`. Invariant: **replay_max_events ≤ ws_out_queue_cap − control_slack** (e.g. 4096 ≤ 4608−512) — prevents a new subscriber from overflowing the queue and being disconnected by replay alone.
- JS representation of the seq u64 (number has a 2^53 limit) — **user decision** (currently mapped to number).

### Step 4 — engram-dashboard-daemon bin
- `#[tokio::main]`. `Local\EngramDashboardDaemon-{user_sid_hash}` named mutex (ERROR_ALREADY_EXISTS = already running). bind 127.0.0.1:0 → daemon.json{port,pid,token,protocolVersion} atomic (persistence tmp+rename).
- Owns `Arc<AgentManager>`. Blocking work such as spawn_agent/restore_all goes through **spawn_blocking** (mandatory because restore_all has sleep + a 3s window).
- Per connection: `accept_hdr_async` (Origin allowlist, before upgrade) → Auth frame within 1s (close if absent) → `ws.split()`.
  - **read_task** (independent): parses AgentCommand JSON → calls the manager. On Subscribe, registers a WsOutputSink (conn_tx clone) + records it in the (agentId,SinkId) registry. A client binary frame is a protocol-error close. **read is independent of write backpressure** (Gemini's deadlock concern is resolved here automatically).
  - **write_task** (independent, single writer): receives `WsOutbound::{Text(AgentEvent), Binary(frame), Close}` from conn_tx (bounded) and writes to SinkHalf. SubscribeAck/StatusChanged/AgentListUpdated/RestoreResult all pass through this queue as Text (order preserved).
  - **supervisor**: when either read/write ends, cleanup_connection (iterate the registry and unsubscribe everything).
- **WsOutputSink**: sync `send(&OutputChunk)` → `encode_terminal_frame` (raw) → conn_tx.try_send. Full→Err (the core removes the dead sink) + `mark_backpressured` (conn closing swap + Close signal = clean up that whole connection).
- **WsStatusSink** (GPT): status_changed/agent_list_updated/restore_result → `WsOutbound::Text(AgentEvent::...)` try_send. Without it the list cannot be updated.
- **WMI 2-layer (Claude)**: WMI Win32_Process.Create is **only for the Tauri→daemon spawn** (bypasses breakaway). Daemon→PTY child keeps the existing portable-pty + `job.assign` (platform/windows.rs) as is — no WMI/CREATE_SUSPENDED needed. If AssignProcessToJobObject fails, the child is killed immediately + Err (GPT, prevents orphans).

### Step 5 — Tauri→daemon discovery/spawn (DaemonClient is phase4)
Read daemon.json → if absent/stale (pid dead), spawn the daemon via WMI → retrieve port/token. WMI Create requires a fully-qualified exe path + cwd/quoting verification (GPT).

### Step 6 — Isolated harness (hybrid of integration test + harness bin)
- Rust integration test (`daemon/tests/`): codec/auth/subscribe-order/backpressure/multi-agent demultiplexing/unsubscribe on connection close/base64-removal raw regression/epoch mismatch/event-cap.
- Separate harness bin: daemon kill→PTY child cleanup / WMI spawn path / long-running high throughput / ACL cross-user (--ignored). The daemon runs as a child process with assert_cmd+tempfile+tokio-tungstenite.
- Additional cases (GPT/Gemini): Auth timeout 1s / reject if Auth is binary / malformed JSON / oversized frame / protocol_version mismatch / Stale Port Hijacking / Graceful Shutdown (StopDaemon) / Malicious Flood (junk binary before auth).

## Unresolved — user decisions
1. **How to guarantee the SubscribeAck order**: (a) the daemon's single conn_tx enqueue order (keeps zero core changes) vs (b) add `SubscriptionSink::send_subscribe_ack` to OutputCore (more robust but changes the core). Recommend (a) (principle of minimal core changes).
2. **JS representation of seq u64**: number (current, 2^53 limit) vs bigint/string.
3. **port.json ACL strength**: explicit DACL (current user+SYSTEM+Administrators vs current user only) vs relying on LOCALAPPDATA inheritance. The same SID cannot be blocked by ACL, so the real benefit is limited — **a separate security-review area**.
4. **Send queue model**: a single conn_tx per connection (simple, HOL possible) vs per-agent queues + select! multiplexing (mitigates HOL). Recommend the latter (keeps the single writer).
5. **Measure the actual Origin** string of Tauri WebView2 (to register in the allowlist).
