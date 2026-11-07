# S12 Daemonization — detailed implementation design (consult cross-verification merged, ready to start implementation)

Date: 2026-06-14. Basis: two `/consult` rounds (real library survey → critique of the detailed design) GPT·Gemini·Claude-Opus blind + judge. Raw material: `agents/web-runner/shared/20260614-104428-consult-daemon-detail-design/`. Library survey: `ipc-library-consult.md`.

This document is a final design that **merges only the positions confirmed as correct by the decision**. For contradictory issues, the side the judge asserted is adopted.

## 0. Confirmed premises (no re-debate)
- Path B-direct (JS↔daemon direct WS, no relay), replay = daemon in-process (reuse OutputCore), two-mode toggle (Embedded/Daemon, chosen at startup), Cargo workspace static linking, raw tokio-tungstenite, Windows first.

## 1. Key decisions (contradictory issues — asserted by the judge)

### 1-1. WS lane = a single connection, a single receive loop (no lane split)
- **Do not split control and output into separate WS connections.** TCP/WS ordering is guaranteed **per connection**, so two separate connections have no cross-ordering guarantee, causing a causal inversion where "the Killed status arrives before the last output" → the reliability of ET seq ordering collapses.
- **A single 127.0.0.1 WS connection.** Control events and output on the same connection, and the client handles them in **one receive loop** (relative order preserved).
- The concern that "under a PTY flood, status/ack get stuck behind the backlog" is real, but the fix is not a lane split but **backpressure (§1-5) + frontend throttle (§3-a)**.

### 1-2. Wire codec = custom fixed-header binary frame for the output hot path, JSON for control
- Discard base64-in-JSON (33% inflation + CPU; the current base64 was a workaround for the Tauri Channel JSON constraint — WS supports the binary opcode).
- **output (hot path)**: WS **binary frame**, fixed header `[tag:1][agentId:16][epoch:4][seq:8][raw payload]`. Zero serialization/parsing. `tag` distinguishes the OutputChunk variant (0=TerminalBytes; future API output has low throughput so it uses JSON like control).
- **control (low frequency)**: WS **text frame JSON** (or MessagePack). Simple decoder.
- Keep kind-agnosticism (§2): only the wire representation splits per variant; OutputChunk is not frozen into bytes.
- (Full MessagePack is also acceptable, but a variable-encoding cost remains on the hot path, so it is not recommended.)

### 1-3. Replay starting point + ring truncation + raw≠screen state (★the decisive reinforcement GPT caught)
- epoch **mismatch** = full reset. epoch **match** = resume from afterSeq+1.
- ★**The replay buffer is bounded (a 2MB/agent cap exists)** → `afterSeq < oldestSeq` (pushed out of the ring during reconnect) can happen. **"Replay from seq 0" is physically impossible in some cases.** A branch is mandatory:
  - epoch mismatch → Reset, replay from oldestSeq, `truncated = oldestSeq>0`.
  - epoch match & afterSeq<oldestSeq → truncated replay (from oldestSeq) + UI shows "output truncated".
  - epoch match & afterSeq≥oldestSeq → Resume (from afterSeq+1).
- ★**Raw byte replay ≠ terminal screen state** (alt-screen/cursor position/progress bars/full-screen TUI break when replayed from the middle). **v1 severity: not fatal** — the normal path (subscribe from the start, inside the ring) is exact. Only the exceptional path outside the ring (truncated) degrades.
- **v1 contract stated explicitly:** "Only exact byte resume is guaranteed. If afterSeq is inside the replay window, gap/dup is 0. Outside the window, exact terminal state restoration is not guaranteed → clear + tail replay + show 'output truncated'." VT-parser snapshot/disk spool is v2.
- ★**A separate case where screen restoration can break even with truncated=false**: the `truncated` flag guarantees only "the requested afterSeq is inside the ring (seq gap 0)". But when the ring fills up and evicts from the front, **if the "initial bytes that set the screen state" such as the alt-screen entry sequence (`\e[?1049h`) or a full-screen clear (`\e[2J`/`\e[3J`) are pushed out of the ring**, all later bytes stay inside the ring and are judged truncated=false, yet screen restoration still breaks (e.g. a full-screen TUI is replayed with only the body and no alt-screen entry sequence). That is, this is a case **independent** of the "seq continuity" the truncated flag guarantees (seq is continuous but the screen-state seed bytes are lost), and a fundamental limit of the raw byte ring model. It is to be resolved by phase 4's VT-framebuffer (the VT parser keeps the screen state as a snapshot → the screen itself is restored on reconnect).

### 1-4. Token delivery = ACL port file only (no arg·query string·env) ★updated after spike #1
- A process command line is **readable by other processes of the same user** via `wmic process`/`Get-CimInstance Win32_Process` → a CLI arg token is exposed. A query string also leaves traces in proxies/logs (low risk because it is loopback, but not zero).
- ~~env~~ is also discarded: **WMI `Win32_Process.Create` (the spawn adopted in spike #1) cannot inject environment variables** → the env path simply does not work for the daemon.
- The token goes only through an **ACL-locked port.json (current user only)**. The daemon generates a 256-bit token at boot → writes it to port.json (ACL) → Tauri reads it and submits it as the first Auth frame. Verification is the **first Auth frame right after connecting** (close if none within 1 second).

### 1-5. backpressure = bounded + emit try_send only + disconnect the slow consumer (zero core change)
- If the producer (pump) **awaits** on a slow client, PTY reads stop → the agent halts. `OutputSink.send` must be **non-blocking (try_send)**.
- A **bounded mpsc** per subscriber. When full → that subscriber is **marked dead + the socket closed** (= isomorphic to the current dead-sink removal mechanism at `output_core.rs:110-123` → **zero core change**). Recovery by reconnect + afterSeq replay (the 2MB ring is the safety net).
- No dropping (seq continuity, ANSI corruption), no unbounded buffer (OOM). The answer is "disconnect and recover by reconnecting".

## 2. Module/crate layout (Cargo workspace, static linking)
```
engram-dashboard/ (repo 루트 = workspace)
  crates/
    engram-dashboard-protocol/  [신규] AgentCommand·AgentEvent·OutputChunk·Hello·ID — serde, Tauri import 금지
    engram-dashboard-core/      [이동] pty/ 전부 + persistence + logging. AgentManager·OutputCore·AgentTransport·backend. Tauri import 0
  engram-dashboard-daemon/      [신규] main: lock/portfile·WS server·auth/version·AgentManager 소유·PTY Job 소유
  src-tauri/                    [얇아짐] Embedded: core 직접 / Daemon: discovery·spawn만 + AgentClient 모드 전달
  src/ (frontend)               AgentClient 인터페이스 신설, 컴포넌트·스토어는 인터페이스만 의존
```

## 3. protocol (engram-dashboard-protocol — linchpin)
- `AgentCommand` (UI→core): Spawn{profileId}·Kill{agentId}·Interrupt·WriteStdin{agentId,data,**requestId**}·Resize{agentId,cols,rows,viewportId}·Subscribe{agentId,epoch?,afterSeq?}·Unsubscribe·ListAgents·StopDaemon{force,killAgents}·Profile(CRUD).
- `AgentEvent` (core→UI): Hello{protocolVersion,daemonVersion,capabilities}·Ack{requestId}·SubscribeAck{action(Reset|TruncatedReplay|Resume),currentEpoch,oldestSeq,latestSeq,replayFrom,truncated}·Output{agentId,epoch,seq,chunk:OutputChunk}(binary frame)·ReplayComplete·StatusChanged{agentId,status,epoch}·AgentListUpdated{agents}·RestoreResult·Error.
- `OutputChunk`: TerminalBytes(serde_bytes Vec<u8>)·TextDelta·Usage·ToolCall·Structured — kind-agnosticism kept.
- Type generation: ts-rs (stable) vs tauri-specta (integrated, RC risk) — **user decision**.
- **command idempotency**: `requestId` on every side-effect command. The daemon keeps a short-TTL dedup table. **No automatic retry** (writeStdin duplicate = duplicate input). If the connection drops after send and before ack → QueryCommandResult(requestId) after reconnect.

## 3-a. Frontend (AgentClient)
- Common interface: `subscribe(agentId,onEvent)` + `onConnectionStateChange(connected|reconnecting|down)`. spawn/kill/interrupt/writeStdin/resize/unsubscribe/listAgents.
- EmbeddedClient: invoke/Channel as is, connectionState always connected, dedup is a no-op.
- DaemonClient: WS + exponential backoff reconnect + automatic Subscribe{epoch,afterSeq:lastReceivedSeq} on reconnect + seq dedup (absorbs 1~2 duplicates at the reconnect boundary). Reconnect·afterSeq·dedup are **isolated internally** (not raised into the interface — avoids the Embedded LSP smell).
- **Right after connecting, the daemon pushes a full agent list snapshot** (prevents losing AgentListUpdated during reconnect — correctness of the terminal decision).
- ★**Frontend render throttle is mandatory**: if high-speed PTY output flows straight into React, virtual DOM rendering falls behind and the tab freezes/OOMs. Chunk per rAF/16ms. **Who does it is a decision (user)**: daemon-side 16ms bundling vs frontend rAF flush.
- ★**multi-client ControlLease**: when desktop+mobile attach to the same agent at once, there is only one PTY size → writeStdin/resize/interrupt only from the lease owner (ControlLease{agentId,ownerClientId,viewportId,expiresAt}). Separate read-only viewers.

## 4. Daemon lifecycle (Windows)
- ★**spike #1 done (2026-06-14) — result: `spike1-breakaway-result.md`. Decision GO, but the spawn method changes.**
  - The measurement environment (IDE/CLI shell) parent Job = `0x2000` (KILL_ON_JOB_CLOSE + breakaway not allowed) = worst case.
  - `CREATE_BREAKAWAY_FROM_JOB` directly → **fails with os error 5.** `cmd /c start /b` fallback → **dies together (design assumption discarded).**
  - **WMI `Win32_Process.Create` → in_job=false, detachment succeeded** (WmiPrvSE parent, does not inherit the caller's Job).
- **spawn = adopt WMI `Win32_Process.Create`** (or adaptive: query the Job flags → not KILL_ON_JOB_CLOSE = normal / breakaway_ok = CREATE_BREAKAWAY / worst case = WMI). The daemon owns its own lifetime.
- ★**Derived constraint: WMI Create cannot inject env** (CommandLine only) → **token delivery = ACL port.json enforced** (the §1-4 env path discarded). Depends on the winmgmt service (always running).
- ★**"Job ownership transfer" is an illusion — deleted.** A Job handle is process-local and cannot be transferred. In daemon mode the daemon spawns the PTY directly, so putting it in its own Job (KILL_ON_JOB_CLOSE) is all there is. The spawner differs by mode, so there is no runtime transfer scenario.
- Single instance: **`Local\` named mutex** (current user only — safer than `Global\`) + an ACL-locked `%LOCALAPPDATA%\Engram\daemon.json`{port,pid,token,protocolVersion} atomic write (reuse persistence tmp+rename).
- Discovery: **bind 127.0.0.1:0 (random port)** → port.json. Tauri reads it and connects to /health. No fixed port (collisions·stale).
- Boot: check whether the pid of a stale port file/mutex is dead, then clean up.
- Shutdown: survives UI close (the purpose). `StopDaemon` command (§5 LLM control). **idle-timeout default is a user decision** (C=OFF / Gemini=30 min; compatible proposal: self-terminate when "PTY 0 + connections 0" persists for N minutes, live forever if there is an active agent).
- **protocolVersion negotiation**: the daemon has a deployment lifetime independent of the app → check the version with Hello. incompatible → StopDaemon(old)+new spawn (automatic vs user confirmation = a decision).

## 5. Security
- Bind 127.0.0.1 only (no 0.0.0.0/LAN, before mobile is turned on). 256-bit token (env/ACL port.json). Verify the first Auth frame. **Origin allowlist** (Tauri origin). Message size/rate limit. permessage-deflate off.
- Named pipe (OS-level restriction via ACL) is a v2 option for hardening desktop security. Mobile v3 = wss/TLS + pairing + device allowlist.

## 6. Implementation phasing
- **★spike 0 (before phase 1):** Measure Job Object breakaway. Whether detachment is possible when the parent job (IDE/terminal) disallows breakaway + verify the fallback (`cmd /c start /b`). **If it does not resolve, everything is void.**
- **✅ phase 0 (done 2026-06-14):** an independent engram-dashboard-protocol crate. AgentCommand/AgentEvent/OutputChunk/SubscribeAction + domain mirror + codec binary frame + ts-rs bindings. 21 tests PASS (codec golden/roundtrip/errors + per-type export). Commit `61f2d0f`. Note: name collision (protocol AgentCommand ↔ core profile AgentCommand = spawn kind) → rename the spawn kind to SpawnSpec in phase 1+.
- **✅ phase 1 (done 2026-06-14):** 
  - **1a backend:** Cargo workspace (root Cargo.toml: protocol/core/src-tauri). pty/persistence/logging→`crates/engram-dashboard-core` (git mv, history preserved, internal crate:: untouched). examples also moved to core. Isolation gate use tauri 0. Zero regressions (core unit 38 / headless·transport_smoke·session_smoke / full build / tauri dev OK after relocating target to the workspace). Commit `576c5e1`.
  - **1b frontend:** AgentClient interface + EmbeddedClient (encapsulates Channel·base64·#13133) + clientFactory (singleton + §5 window.__ENGRAM_AGENT__). Components ptyApi→agentClient. GUI E2E: spawn→subscribe→writeStdin→decoded 277B/3 chunks, 0 after kill, UI normal. Commit `5346240`.
- **phase 2:** Daemon alone + isolation harness (extend transport_smoke). Required cases:
  - Lifecycle: single instance / stale port·lock / incompatible version / **UI kill→daemon survives** / **daemon kill→PTY child cleaned up** / duplicate spawn race / **breakaway success confirmed**.
  - Security: reject no token·wrong token·Origin mismatch·oversized·auth timeout.
  - Output: order exact / afterSeq boundary off-by-one / afterSeq==latest / **afterSeq<oldest (truncated)** / epoch mismatch / live race during replay / **C4 gap·dup 0** / slow consumer disconnect / reconnect recovery / ring memory cap.
  - PTY: high throughput / resize flood / interrupt / child exit·crash / stdin close.
- **phase 3:** Keep the harness permanently in CI (protocol golden / core contract / embedded contract / daemon contract / Windows lifecycle).
- **phase 4:** Swap at the seam. startup config mode=embedded|daemon → AgentClientFactory. No live hot-swap.
- **acceptance test:** Tauri restart → the daemon survives + lossless restore.

## 7. User decisions needed (unresolved)
1. ✅ **idle-timeout = OFF, always alive** (decided 2026-06-14). Reason: a scheduler may later wake agents, so the daemon must always be alive. No idle auto-shutdown. (Terminated only by the explicit StopDaemon command.)
2. ✅ **Type generation = ts-rs** (decided 2026-06-14). Reason: all that is needed is automatic generation of the protocol types (AgentCommand/AgentEvent). tauri-specta's invoke-binding advantage is void on the WS daemon path + specta 2.0 RC is unstable. ts-rs alone covers the types for both modes. For the u64 seq, consider a `#[ts(type="bigint")]` or string mapping (JS number 2^53 limit).
3. **truncated replay UX** — tail+marker (v1) vs VT snapshot (v2) vs disk spool. v1 scope.
4. **Local transport** — TCP+token alone (v1) vs named pipe+TCP dual (at hardening time).
5. **Who does the frontend throttle** — daemon 16ms bundling vs frontend rAF flush.
6. **Old daemon on version mismatch** — automatic termination vs user confirmation.
7. **Multiple Tauri instances** — policy for allowing multiple windows on a single daemon (confirm the double-spawn guard at manager.rs:90 is kept in the daemon).

## 8-b. Decision log (Q&A in progress 2026-06-14)
- **#1 idle-timeout = OFF, always alive** (in preparation for waking agents via a scheduler). Only an explicit StopDaemon.
- **#2 Type generation = ts-rs** (it is WS, so tauri-specta's invoke advantage is void; avoids RC).
- **#3 truncated replay = A (clear+tail+marker)** + **buffer = configurable default, adjusted at runtime by the agent (LLM)** (SetReplayBufferSize). **Structured output (per turn)**: essential at its core, but **designed later as a TUI↔structured switching structure**. claude is a choice between an interactive TUI (bytes, no unit boundaries) and `-p stream-json` (structured turn/tool, gives up the TUI) — not an SDK. The daemon only **leaves open** that OutputChunk can represent both (TerminalBytes / a structured variant + turnId); the implementation is at switching design time.
- **#4 epoch mismatch = full reset.** backpressure = bounded queue + **emit try_send (never block = zero core change)** + when it overflows **disconnect only that client** (no watermark pause — a plain disconnect model). **Recovery = client-driven, by the cause of the disconnect:**
  - When the server disconnects because of the buffer → WS close reason tag (`SlowConsumer`) → the client **requests a refresh** (jumps to the present without receiving the piled-up backlog).
  - A plain error (abnormal close) → the client **resume (afterSeq)** → the server continues if it is still in the ring, and otherwise **degrades to refresh**.
  - The "current state" a refresh provides: v1 = clear+tail (approximation), v2 = VT parser screen snapshot (exact, later — a far-behind refresh is a clear motivation for v2).
  - Principle: **server = autonomous buffer policy + status notification, client = resume/refresh decision.** A thrashing client can be **rate-limited/blocked by the server (later)**.
  - **Location of the recovery logic: separated into DaemonClient** (a network disconnect exists only in Daemon). Embedded has zero recovery. Behind the shared `AgentClient` interface → it can be filled in later only inside DaemonClient (no effect on the shared part or Embedded). **Detailed implementation later.**

## 8-d. The 7 user decisions — all done (2026-06-14)
1. **idle-timeout = OFF, always alive** (in preparation for waking agents via a scheduler). Only StopDaemon.
2. **Type generation = ts-rs.**
3. **truncated replay = A (clear+tail+"truncated" marker).** Buffer = configurable default + runtime adjustment by the agent (LLM). Structured (per-turn) output = designed later as a TUI↔structured **switching mode** (§8-c); the daemon only leaves OutputChunk open for both representations.
4. **epoch mismatch = full reset.** backpressure = bounded + emit try_send (zero core change) + handling for a client that cannot keep up — **disconnect vs flow-control pause (client ACK): pick one at DaemonClient implementation** (the standard is pause). Recovery = **client-driven**: by the cause of the disconnect (server close reason `SlowConsumer`→refresh / abnormal→resume); the server degrades to refresh when resume is impossible. The refresh "current state" = v1 clear+tail / v2 VT snapshot. Recovery logic = separated into DaemonClient (Embedded recovery 0). **Detailed implementation later.**
5. **throttle = standard**: the daemon relays read chunks (no special batching), and the client renders at xterm frames (rAF, skip-frame, 50MB). Adaptive/frame-skip·sync mode·flow control are all provided by xterm. (Verified: ttyd/gotty relay + xterm.js rAF + ordinary terminals' vsync rendering.)
6. **Version handling = much later (deferred).** Not built now (both are launched together). Later: protocolVersion (+1 only when it breaks) **or/and** a **popup guide** on parse-error (the user restarts the daemon) — no auto-restart·negotiation layer is built.
7. **The client = 1 (the Tauri app = 1 Rust backend) + several React windows (views).** Shared state (layout·moving slots between windows·the management window singleton) lives in one place in the backend, and windows are views (Tauri windows are independent React, so sharing goes through the backend/events). Multi-subscriber is one connection per client (app·mobile), not per window. **Multi-window details + multi-.exe policy = later.**
- **transport = v1 TCP (127.0.0.1)+token.** Named pipe = v2 security-hardening option. (The mobile goal is direct WS, so TCP is unified.)

**→ Design decisions complete. Next is the implementation stage. ★Top priority: the Job Object breakaway spike (the single point of failure for daemonization) — measure alone before phase 1.**

## 8-c. Mode switching architecture (implement later, build then user review)
Two orthogonal axes: **location** (Embedded/Daemon = AgentClient) ⟂ **mode** (TUI/Structured = AgentMode). Principle: **mode = a bundle of [how to run + transport + capability + renderer]; the core (OutputCore/Session/Manager/AgentClient) is mode-agnostic.**
- Backend: `AgentMode` enum→dispatch. `ClaudeTui` (interactive CommandSpec / PtyTransport / cap.output=terminal_bytes / emit TerminalBytes) vs `ClaudeStructured` (`-p stream-json` CommandSpec / **StreamJsonTransport** (new, parses stdout JSON lines) / cap.output=structured / emit TextDelta·ToolUse·ToolResult·Usage). Both use the same OutputCore.
- Frontend: the Slot picks a renderer by `capability.output` — terminal_bytes→TerminalRenderer (xterm, existing) / structured→**StructuredRenderer** (turn vector·folding·VS Code style, new). StructuredRenderer serves both claude-stream-json and the future HTTP API.
- Already present (S10): the AgentTransport trait · OutputCore variant-agnostic emit · the OutputChunk/Capabilities slots · backend dispatch. New: AgentMode·StreamJsonTransport·StructuredRenderer·turnId/exchangeId.
- Switching = decided at spawn time (no live conversion — TUI vs stream-json are different process executions). A mode change = respawn in that mode (with --resume if needed; whether a resume with a changed output mode works is to be measured). Zero rewrite — adding a mode = 1 transport + 1 renderer + a bundle.

## 8. Contribution per model (reference)
- Claude-Opus (most trusted, zero factual errors, verified the code directly): single WS·binary frame·try_send=zero core change·"Job ownership transfer = illusion"·breakaway single point of failure·list snapshot·version negotiation.
- GPT (decisive reinforcement): ★ring truncation truncated branch·raw≠screen state·command idempotency·control lease·named mutex-alone risk.
- Gemini (caught alone): ★frontend render throttle (rAF)·zombie daemon idle-timeout.
- Rejected: GPT's control/output lane split (no cross-ordering guarantee), Gemini's "always seq 0" and allowing the token as arg/query.
