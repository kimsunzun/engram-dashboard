# S17 control channel — consolidated research report

> Written: 2026-07-16  
> Scope: persists the key findings of the two independent investigations (the main `/research` fan-out + the user's separate Codex investigation) that ended the two-day control-channel debate.  
> **Final decision source of truth = [ADR-0086](../../decisions/0086-제어-채널-듀얼-typed-입구mcpcli-sqlite-메일박스-first-마커m3-폐기.md).** The purpose of this document is to preserve the research content that was the basis of the decision.  
> Original Codex investigation: `I:\claude-code-multiagent-messaging-research.md` (outside the repo — at risk of loss, so the key content is recorded in this document).

---

## 1. Research question

What is a proven pattern for the channel that carries agent A→B messages and control commands? Specifically:

- Does a mature OSS implementation exist that uses a free-text marker (in-band output marker, M3) as the main channel?
- Is there a case that used the `PreToolUse` hook as the main transport path?
- What is the ecosystem-dominant pattern, and how does it fit engram-dashboard's separated daemon structure (ADR-0029)?

---

## 2. Method — two independent investigations + a cross-family confirmation structure

### 2-1. Main-session `/research` fan-out

The main Claude Code session spawned subagents for 3 branches in parallel with the `/research` skill. Each branch investigated, at code level, a different sample of the ecosystem (Orca, claude-squad, Claude Swarm, Agent Teams, MCP Channels, Tmux-Orchestrator, claude-flow, etc.).

### 2-2. The user's separate Codex investigation (independent)

The user ran Codex separately, unrelated to the main investigation. Codex's `web_search` tool was not exposed at that point (`NO_NATIVE_WEBSEARCH`), and it analyzed the same OSS sample set by reasoning alone. Cause not decided — **"it was not confirmed whether the missing web_search exposure is a configuration defect or a Codex build regression" (uncertain).**

### 2-3. Cross-family confirmation established

The two investigations ran independently without knowing of each other and converged on the same key conclusion. This established a cross-family confirmation, and the **conclusion grade = certain** was raised accordingly. In addition, Codex corrected 2 items of the main investigation (see §4).

---

## 3. Converged conclusions

The conclusions both investigations reached in common are as follows.

- **Mature OSS implementations that adopted free-text-marker stdout scraping as the main channel = 0.** It can be used for observation signals that can tolerate loss (telemetry, exit signals), but it is unsuitable as the source of truth for routing and control messages.
- **Implementations that adopted the `PreToolUse` hook as the main transport path = 0.** A hook suits auxiliary functions such as observing tool execution, security checks and inbox notifications, but it does not fire if the agent calls no tool, so it cannot be an arbitrary inbound message transport.
- **Raw `tmux send-keys` relay = anti-pattern.** There is no typed contract and misdelivery occurs.
- **The ecosystem-dominant pattern = a typed dedicated interface + a durable mailbox.** Orca (20.1k⭐) demonstrates it with a combination of a Bash carrier + typed CLI + SQLite WAL mailbox.
- **MCP Channels are not for agent-to-agent messaging.** Because of their ephemeral nature + delivery-reliability issues (GitHub issue #45563) they are unsuitable as an agent-to-agent control channel.

---

## 4. The 2 items where Codex corrected the main investigation

Following the user's instruction, Codex's judgment took precedence in the corrected items.

### Correction ①: Orca's transport mechanism

**Understanding in the main investigation's draft:** it could be read as Orca using an output marker (magic string).

**Codex correction:** Orca (20.1k⭐, v1.4.143) uses a **Bash carrier + typed CLI**. The agent runs structured `orca orchestration send/reply/ask/check` CLI commands in the ordinary Bash tool. "Running a typed CLI through Bash" is a strong pattern, and only "smuggling a magic string through stdout" is 0. These two concepts must not be confused.

Reference permalinks (included in the original Codex investigation):
- DB schema: <https://github.com/stablyai/orca/blob/8c1d13dc52a84b0b38213fe2e76e1a84780b206e/src/main/runtime/orchestration/db.ts#L82-L115>
- send/fan-out/delivery: <https://github.com/stablyai/orca/blob/8c1d13dc52a84b0b38213fe2e76e1a84780b206e/src/main/runtime/rpc/methods/orchestration.ts#L207-L283>
- ask/reply: <https://github.com/stablyai/orca/blob/8c1d13dc52a84b0b38213fe2e76e1a84780b206e/src/main/runtime/rpc/methods/orchestration.ts#L365-L385>

### Correction ②: Whether the Agent Teams file mailbox is official

**Main investigation's draft:** described the `~/.claude/teams/<team>/inboxes/<agent>.json` file mailbox structure as how Agent Teams works.

**Codex correction:** this structure is an **unofficial reverse-engineered observation** of Claude Code 2.1.45. The internal source is not public, and there are also reports of tool/model integration changes during 2026, so behavior may differ depending on the installed version. **It must not be regarded as an official API contract.**

---

## 5. Key insights

### "MCP or Bash" is a secondary question

The key insight Codex drew: the real fork is not the transport means (MCP vs Bash).

> **"Does the orchestrator own the typed ingress + durable mailbox?"**

- Strong pattern: typed ingress + durable mailbox + optional push
- Weak pattern: best-effort injection that depends only on a free-text marker or terminal state

Even when Bash is used as the transport entry, it is a strong pattern if the payload and error contract are structured (Orca demonstrates this).

### Durable mailbox ownership — 3-state separation

The `sequence` / `delivered_at` / `read` (boolean) separation in Orca's design is highly worth adopting:

- `sequence`: deterministic ordering (SQLite autoincrement, monotonic per recipient)
- `delivered_at`: prevents re-injecting a message already pushed to the PTY/context
- `read`: separately marks whether the consumer actually consumed the inbox — **Correction (2026-07-19, verified against the real source):** the initial investigation recorded it as `read_at` (a timestamp), but the actual schema is a boolean flag `read INTEGER NOT NULL DEFAULT 0` (`db.ts` @8c1d13dc — verified through the commit-pinned permalink). Not a timestamp.

By separating these three states, "put into the terminal" / "delivered to the context" / "read by the agent" can be tracked independently.

### Two-level ACK

The ecosystem-dominant pattern separates two levels of ACK:

1. **Synchronous enqueue ACK** — confirms only up to "it was put in the mailbox" (recipient exists · schema valid · DB append succeeded · message ID returned)
2. **Asynchronous semantic reply** — a separate reply message / thread response / worker result

"The other side has read it" is not required of the synchronous ACK.

### Structural advantage of Engram's separated daemon

Orca is an Electron-app-embedded runtime and dies with the app. Engram's separated daemon (ADR-0029) keeps agents and the mailbox alive even after the client (the src-tauri shell) detaches, so it has a structural advantage in recovery semantics. The structure "the daemon owns the PTY and the client is detachable" is a classic pattern the tmux server has demonstrated for a long time.

---

## 6. Spike measurement summary (5 items)

①~④ were run in the research session, ⑤ right before step 1 started (2026-07-16). The spike harnesses were deleted as throwaway. They can be reproduced under the conditions below if needed.

### Spike ①: stream-json envelope structure

Analysis of envelope types when running `claude -p --output-format stream-json --verbose`.

**Findings:**
- In the `complete` envelope `tool_use.input` is fully formed (no delta reassembly needed).
- `partial_json` occurs only with `--include-partial-messages`.
- There are more envelope types than expected: `user` / `rate_limit_event` / `system:thinking_tokens` / `system:status` / `stream_event` — **defensive parsing is mandatory**.
- `result` = process completion (one-shot), the final `stop_reason` value appears only in `message_delta`.

### Spike ②: Bash sentinel compliance (haiku model)

Measured how well the model complies with a structured Bash marker.

**Findings:** 4 of 6 runs parsed clean (66%). Failure causes: broke on a body containing quotes / nonce misrecorded once / asked back on an ambiguous recipient. **This number is the decisive measured basis for discarding the marker approach.** Even with a strict instruction, 66% is hard to trust as a main channel, and it falls lower with weaker models and long sessions.

### Spike ③: hook mechanism (confirmed from documentation)

**Findings:** the `PreToolUse` hook fires OK in headless (`claude -p`). But there is an intermittent idle bug #24327 on a `deny` response. This also confirms discarding the hook-as-main-channel approach.

### Spike ④: possibility of dynamic MCP loading (confirmed from documentation)

**Findings:** an MCP server must be declared at startup (immutable after `system:init`). `tools/list_changed` is supported, but that is a change of the toolset of a connected server, not dynamic addition of a new server. The cost of a dormant MCP server is negligible (`tool search deferred`, default from v2.1.191+ — only names are loaded). **"Connection succeeded" becomes a milestone verifiable right after spawn.**

### Spike ⑤: actual transmission of the mcp-config Bearer header (measured on claude 2.1.170)

The web research found many bug reports of the header being dropped (#48514 open · #50464 closed as not-planned · #59467 OAuth takes precedence · #32191 headless silent exit — in the v2.1.71~140+ range), which hit a stop condition, and with the user's approval it was decided by a local measurement.

**Method:** brought up a minimal streamable HTTP MCP response server (node, 127.0.0.1:9876) that logs every header, and ran `claude -p --allowedTools mcp__engramspike__spike_ping` with an mcp-config containing `headers: {"Authorization": "Bearer <토큰>"}`.

**Findings (all green):**
- **The Authorization header is sent on every request** — initialize · notifications/initialized · SSE GET · tools/list · tools/call, all of them. Sending was confirmed even on the `(sdk-cli)` user-agent path where #50464 reported it dropped.
- The client echoes the `Mcp-Session-Id` handed down by the server on every following request → the design that binds token→session at initialize time is valid.
- The headless silent exit (#32191) did not reproduce — E2E success through the tool call (`pong` received). Protocol version `2025-11-25`.

**Implication:** the step 1 design "pass the token by Bearer header" is confirmed to stay. But **our daemon MCP server must not advertise OAuth metadata** (#59467 — when advertised, the static Bearer is ignored).

---

## 7. Ecosystem case summary

### Orca (★★★ — representative of the dominant pattern)

- Stars: about 20.1k (as of 2026-07-16)
- Transport mechanism: the agent runs the typed CLI `orca orchestration send/reply/ask/check` in Bash
- Delivery: central Orca runtime → SQLite WAL mailbox
- Mailbox fields: `sequence`, `read`, `delivered_at`, `thread_id`, `id` (unique index)
- ACK: `send` returns the message ID/number of recipients. `ask` supports synchronous request-response
- Recovery: restart recovery is possible with SQLite WAL + a persistent DB
- Limit: an exactly-once guarantee was not confirmed (uncertain)
- Structure-reference permalinks: see §4 correction ① above

### mcp_agent_mail (reference — MCP send_message semantics)

A reference implementation of the MCP `send_message` tool semantics. Listed in the handoff as a reference.

### awslabs CAO (reference — env identity pattern)

A case demonstrating the environment-variable identity pattern at the CLI entry (e.g. `CAO_TERMINAL_ID`). The reference basis for engram's design of injecting the `ENGRAM_TOKEN` env.

### claude-flow / Ruflo v3 (reference — caution needed)

- Stars: about 64.5k (as of 2026-07-16)
- The internal `MessageBus` is a per-agent in-memory priority queue + EventEmitter structure in a central process.
- There is an `enablePersistence` setting, but Codex could not confirm a real persistence connection in the code.
- A defect suspected in the retry logic: code in `addToQueue()` that re-creates `attempts` as 0 → the retry bound may not work as intended. **This is a suspected defect in code, not confirmed (uncertain).**
- **Decision: do not reference:** several generations of implementations coexist and there are many placeholders. Code reliability is low per Codex's review.
- Reference permalink: <https://github.com/ruvnet/ruflo/blob/a0c1ac4b4ff84360cb85b577e1da81eb661a078f/v3/%40claude-flow/swarm/src/message-bus.ts>

### MCP Channels — Anthropic (reference — excluded for agent-to-agent)

- Official research preview, Claude Code v2.1.80+
- inbound: the MCP server pushes a `notifications/claude/channel` notification
- outbound: the model calls the channel MCP reply tool
- **Limits:** events arrive only while the session is open. No built-in durable queue/replay. ordering/dedup/restart recovery are the responsibility of the channel server or an external platform.
- For agent-to-agent messaging it was judged unsuitable because of its ephemeral nature + the delivery-reliability issue in GitHub issue #45563.
- Official docs: <https://code.claude.com/docs/en/channels-reference>

### Tmux-Orchestrator family (anti-pattern)

- Raw `tmux send-keys` + sleep: no delivery guarantee (it is merely keyboard injection).
- Improved implementations (literal mode, idle handshake, status file) also have no message dedup/transactional replay.
- **Decision:** can be used only in a limited way as an active-session wake-up transport. Cannot be a source of truth.

### Claude Swarm v1 / parruda (reference — hierarchical RPC pattern)

- Exposes the other agents as MCP tools/servers to each Claude Code instance. A delegate MCP tool call → the target `claude -p` process runs → the result returns as an MCP `tool_result`.
- Suited to hierarchical RPC where A delegates work to B and waits for just one result.
- A different pattern from a long-lived mailbox or free peer chat.
- The latest pinned v1 SHA **could not be obtained** because of repo redirect/clone instability at research time (uncertain).

---

## 8. Final decision pointer

The final decision made on the basis of the research above is recorded in **[ADR-0086](../../decisions/0086-제어-채널-듀얼-typed-입구mcpcli-sqlite-메일박스-first-마커m3-폐기.md)**.

Summary: **dual typed ingress (MCP `McpIngress` + CLI `CliIngress`) + a common `ControlIngress` seam + SQLite WAL `Mailbox` (source of truth) + `Dispatcher` push.** The choice of default ingress (MCP vs CLI) is decided by the user after the step 6 A/B measurement.

The earlier decision ADR-0086 discarded: ADR-0085 (marker = main channel — discarded entirely).

---

## 9. Source list

1. **Original Codex investigation (primary):** `I:\claude-code-multiagent-messaging-research.md` — outside the repo, at risk of loss, so the key content is recorded in this document. Research reference date 2026-07-16.
2. **Session handoff:** `.claude/handoff/latest.md` — records the convergence of the two independent investigations, the 2 Codex corrections, the 4 spikes and the architecture approval state.
3. **ADR-0086:** `docs/decisions/0086-제어-채널-듀얼-typed-입구mcpcli-sqlite-메일박스-first-마커m3-폐기.md` — the final decision source of truth.
4. **M3-era review note:** `docs/process/S17-llm-control-surface/control-channel-deliberation-m3.md` — snapshot of the issues and measurements when ADR-0085 was decided (now superseded).
