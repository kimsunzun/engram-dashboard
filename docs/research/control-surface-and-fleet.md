# Research note: claude control surface + fleet upfront design

- Nature: **research note (fact gathering)** — not a decision. Decisions go separately into a PRD/ADR. Upfront reference for when design enters "control surface / fleet" after the backend abstraction (S10) ends.
- Date: 2026-06-21 · Scope: programmatic control of the claude CLI + hosting many (20+) agents.
- Confidence labels: **[F]** official docs/code confirmed · **[2]** secondary source (blog/forum) · **[?]** unverified/needs measurement · **[op]** opinion.
- Source URLs are at the very end.

---

## 1. claude control surface — 4 channels

Control depth: launch args (outer) → PTY text/slash commands (middle) → **stream-json/SDK (machine control)** → config·hooks (behavior rules).
Engram currently uses only ①+② (PTY spawn + text stdin). The core of fleet/LLM control is ③.

### ① Launch-time (CLI args + env) — fixed per session
Only what bears directly on fleet/programmatic control (the full 30+ flags are in cli-reference.md):
- Session: `--session-id <uuid>`(we control the sid, current) · `--resume <id>` · `--continue` · `--fork-session`
- Model/performance: `--model` · `--effort <low|medium|high|xhigh|max>` · `--fallback-model <list>`
- Permission modes (6): `default` · `acceptEdits` · `plan` · `auto`(classifier auto-review, Opus4.6+) · `dontAsk`(allow rules only, fully automatic for CI) · `bypassPermissions`(skips everything, isolated environments only)
- Tools: `--allowedTools "Bash(git *),Read"` · `--disallowedTools "mcp__*"` · `--tools`(limits what is exposed to the context)
- Output: `-p/--print`(headless) · `--output-format text|json|stream-json` · `--json-schema`(structured validation, requires json) · `--include-partial-messages` · `--verbose`
- Input: `--input-format text|stream-json` · `--replay-user-messages`
- **Fleet essentials**: `--bare`(skips auto-discovery of hooks/skills/plugins/MCP/CLAUDE.md → faster startup, less memory, for CI) · `--mcp-config`+`--strict-mcp-config`(MCP gating) · `--max-turns` · `--max-budget-usd` · `--no-session-persistence`(all `-p` only) · `--exclude-dynamic-system-prompt-sections`(excludes cwd/env → more prompt cache reuse)
- env: `ANTHROPIC_API_KEY` · `ANTHROPIC_MODEL` · `CLAUDE_CONFIG_DIR` · `CLAUDE_CODE_SAFE_MODE`(turns all customization OFF) · `CLAUDE_ENV_FILE`(a hook injects env)
- Limits: runtime only; persistent defaults live in settings.json. session-id accepts only UUID format.

### ② Runtime text stdin — slash commands/mentions (= the current straw)
`/resume` `/clear` `/compact [instr]` `/cd <path>`(CwdChanged hook) `/branch` `/config k=v` `/permissions` · `@file`·`@/dir` mentions · `!bash` · `Shift+Tab`(cycles permission modes).
- Limit: **all interactive-only**. In headless (`-p`), `/resume`·`/clear`·`/cd` are unavailable. Output is ANSI text → fragile to parse (the root weakness of the current xterm scraping).

### ③ Runtime structured — `-p --output-format stream-json` ★core★
NDJSON (line-delimited JSON), bidirectional. Compared with text scraping, event types, metadata (tool_use_id/cost/error), token boundaries and ordering are separated **structurally**.

Outgoing events (main): `system/init`(session_id·model·tools·mcp·plugins) · `assistant`(content_block_delta) · `tool_use`(tool_use_id·tool_name·tool_input) · `tool_result`(matches tool_use_id·output·error) · `stream_event`(token delta, `--include-partial-messages`) · `permission_request`(in headless, cannot be answered directly on stdout) · `system/api_retry`(rate-limit backoff can be implemented directly) · `structured_output`(`--json-schema`) · `result`(final result·session_id·usage·cost).

Incoming messages (`--input-format stream-json`): `{type:"user_message",content}` · `{type:"permission_response",tool_use_id,decision:"allow|deny|ask"}`.
- Limits: tool execution is sequential (no parallel) · permission prompts in headless must be pre-decided by a hook · non-`-p` interactive mode does not support structured input.
- **Implication for engram**: the fork in the road for CLAUDE.md's `OutputEvent` variant-agnostic design (terminal vs structured) is exactly here. If `claude_console` receives stream-json instead of PTY text, we get structured output.

### ④ Persistent config + hooks — behavior rules
- Layers (low→high): user `~/.claude/settings.json` → project `.claude/settings.json` → local `.claude/settings.local.json` → `--settings` → managed. Most reload live (only `model`/`outputStyle` need a restart).
- Keys: `model`/`effortLevel`/`fallbackModel` · `permissions{defaultMode,allow[],deny[],additionalDirectories[]}`(rule syntax `Tool(pattern)`, the most restrictive rule wins) · `env{}` · `enabledPlugins{}`.
- hooks: many events (lifecycle `SessionStart`/`SessionEnd`/`UserPromptSubmit`, tool `PreToolUse`/`PermissionRequest`/`PostToolUse`, `PreCompact`/`PostCompact`, subagent `SubagentStart`/`Stop`, `Stop`/`Notification`, etc.). Types `command`/`http`/`prompt`/`agent`/`mcp_tool`. I/O: stdin JSON, exit `0`=pass·`2`=block(stderr→Claude)·other=error, or `hookSpecificOutput{permissionDecision,updatedInput}` to modify input and decide permission.
- What can be blocked/modified: PreToolUse=block + input modification + permission · PostToolUse=block propagation · UserPromptSubmit/SessionStart=inject context.
- `.claude/`: `commands/` · `agents/`(subagent definitions) · `skills/` · `hooks/` · `.mcp.json`.

### Agent SDK (TS/Py) mapping
- Most CLI flags map to options (`resume`·`model`·`allowedTools`·`permissionMode`·`mcpServers`·`hooks`). hooks are **async callbacks, not shell** (zero spawn overhead).
- Additional SDK control: `on_permission_request`(programmatic approval) · `on_ask_user_question` · direct access to message objects (zero parsing) · async control loop.
- **Decisive fact [F]**: SDK `query()` internally **spawns the claude CLI as a subprocess**. What is in-process in the SDK is the orchestration code, not the LLM loop → memory is the same as headless (`-p`). However, a **custom tool = SDK in-process MCP** removes the MCP child process (see ③).

---

## 2. Fleet hosting (20+ concurrent)

### Core conclusions
1. **Memory floor = number of processes × Node/V8 baseline.** Whether PTY, headless or SDK, if each instance is a separate Node process the baseline does not shrink. The only real saving is subagents (one process), but they **give up isolation**. → **Isolation vs memory is a trade-off** = something for the user to decide in the PRD. [op]
2. **The current PTY + full TUI is the worst combination for a fleet.** Every instance pays the TUI rendering cost (React+Ink/Yoga) + text parsing is fragile. **`-p --bare --output-format stream-json` is the first-choice candidate** — removes TUI rendering + structured output + skips auto-loading, while keeping process isolation. Engram already has the `AgentTransport` seam, so it can be absorbed by adding a "stream-json transport" next to PtyTransport. [op, consistent with ADR-0002/0004]
3. **An MCP process explosion (N×M) can hit before memory does.** An ordinary MCP is an external child process spawned per instance. Mitigations: `--bare`+`--mcp-config` gating / custom tools as SDK in-process MCP / run the MCP as a single shared HTTP-SSE (`type:url`) endpoint and have N instances attach to it [?]. This must be fixed as policy early in the design.
4. **Prompt cache reads are not counted in ITPM** [F] → if 20 instances cache a shared prefix such as the system prompt and CLAUDE.md, rate-limit headroom is secured = effectively raises the fleet concurrency limit.

### Memory composition (interactive 200–400MB, leak peak of 746MB observed [2])
Node/V8 baseline (large, with a pattern of not releasing) + React/Ink/Yoga TUI rendering layer + in-memory context (accumulated over the session) + MCP (a separate process). `-p` does not run the TUI render loop, so it is lighter than interactive. The headless "<256MB" claim [2] — **the exact saving is [?]; RSS needs to be measured directly in engram.**

### Comparison of approaches
| Approach | Memory per instance | Isolation | Controllability | Screen separation | Difficulty |
|---|---|---|---|---|---|
| (a) N full CLIs over PTY (current) | Highest (200–400MB) | Strong (process) | Text parsing (fragile) | Easy | Low (done) |
| (b) N headless `-p stream-json` | Medium (minus the TUI share) | Strong (process) | NDJSON (robust) | Handled separately | Medium |
| (c) Agent SDK in-process, multiple | Same as (b) (= CLI subprocess) | Strong (process) | Type-safe library | Separate | Medium |
| (d) Single process + subagents | Minimum (1) | Context only (no isolation) | Delegation via the Agent tool | Single stream | Low |

Since engram is Rust, using the SDK directly is a poor fit → **changing the current PTY structure to spawn stream-json from Rust effectively combines the benefits of (b)+(c)**. [op]

### Anchors (those that fit engram's constraints)
- **tmux**: server-client split — the heavy screen buffer lives in one server, the client is a lightweight 4MB (100 panes = server 118MB + client 4.2MB). The core is "heavy state in one place, the daemon; display kept thin." [F]
- **Zellij (Rust)**: server-client split, but 22MB idle and rendering 8x slower → **rendering itself is a cost in a fleet; a cautionary example.** [F]
- **ttyd/gotty**: broadcast one PTY to many viewers → the daemon holds one copy of the output, N frontends subscribe. [F, consistent with ADR-0013]
- **Managed Agents (server-hosted)**: zero local processes, but files live in a managed sandbox → does not match engram's "direct manipulation of local files" model. A candidate to revisit if local resource limits are exceeded. [F]

---

## 3. Codebase upfront points (what can be touched before the backend is done)

The seams are already designed to be extension-friendly. Distinguishes what can be done upfront from what needs the backend (file:line are values reported by Explore, approximate anchors):

| Point | Can be done upfront | Needs backend | File |
|---|---|---|---|
| Extend the OutputEvent/InputEvent enum (add variants) | ✅ variant + tests | ✅ actual emit | `core/.../agent/types.rs` |
| Add Capabilities fields (bool, serde-compatible) | ✅ | ✅ fill in values | `types.rs` |
| AgentTransport trait (e.g. `reconfigure`) | ⚠️ add an optional method | ✅ implementations | `agent/transport/mod.rs` |
| AgentCommand variant + backend dispatch | ✅ variant + impl skeleton | ✅ actual behavior | `agent/backend/mod.rs`, `profile.rs` |
| CommandSpec fields (secrets/model_override) | ✅ | ✅ transport reads them | `types.rs` |
| PtyTransport.start pump pattern → mock harness | ✅ frame + tests | ✅ API/SDK transport | `transport/pty.rs` |
| AgentClient methods (`sendMessage`/`reconfigure` stub) | ✅ signatures | ✅ ProtocolClient impl | `src/api/agentClient.ts` |
| ProtocolClient event type design + stub | ✅ | ✅ wire | `src/api/protocolClient.ts` |
| clientFactory `fleet` mode structure | ✅ design (activate later) | ✅ endpoint | `src/api/clientFactory.ts` |

**Possible right now**: ADR draft / extension definitions for OutputEvent·InputEvent·Capabilities·CommandSpec / AgentCommand variant skeleton / AgentClient stub methods / mock transport integration test frame / clientFactory fleet mode skeleton.

**Invariant warnings (breaking them is a regression)**:
- ⚠️ New transport output must always be mapped to an `OutputEvent` variant — no emitting raw bytes directly (OutputCore alone owns seq/replay/fanout).
- ⚠️ Do not reverse the order of the kill 2 verbs `shutdown() → join_pump(5s)` (reversing it hangs).
- ⚠️ Keep the output frame epoch guard + seq dedup.
- ⚠️ Preserve protocol request_id matching / the v2 wire format.

---

## 4. Candidate ADRs (to be recorded when the decision is made)
1. Structured/fleet backend architecture — CLI extension vs headless stream-json vs SDK/daemon. Plan for extending OutputEvent variants. **Make the isolation vs memory trade-off a user decision.**
2. fleet mode selection (clientFactory embedded/daemon/**fleet**) + transport/endpoint.
3. Structured I/O wire format (JSON event vs protobuf, compatible with the current WS binary frame).
4. MCP resource policy (shared HTTP-SSE outside the instance / force in-process) — prevent the N×M explosion.

## 5. Needs measurement (currently [?])
- Actual RSS per headless `-p` instance (the saving versus interactive) · additional saving from `--bare` · output accuracy/regression of the stream-json transport.
- Measurement means: `scripts/cdp.mjs` + integration harness / RSS polling.
- Note: the memory figures in this note (746MB peak, <256MB, per-Tier tokens) are **[2] secondary sources** — measure ourselves before finalizing.

## 6. Open Questions / Deferred

> Basis for the format: the RFC "Open Questions / Future possibilities" (the established standard for unresolved debates) + measurable triggers + ADR-style "rejected alternatives + reasons". The ADR standard has no 'deferred' status, so it is kept here in the research note. Surfacing is reinforced by a related code anchor (`// PARK: …`) — see the items below.

### [PARK] Can a claude.exe warm pool speed up agent boot?
- **Registered item:** status and revisit trigger are managed solely in `docs/tracking.md` **T-9** (prevents rot). This section holds only the *dead ends and the detailed reasoning*.
- **Surfacing anchor (TODO, after the backend settles):** attach one line `// see tracking.md T-9: 프로세스 풀링 — 스폰지연 실측 시 재검토` to the `AgentManager` spawn path → it is exposed automatically when someone touches memory/speed.
- Current conclusion: **pooling is pointless (under the current premises).** Headless (`-p`) is the main path, and retargeting a warm pool to another agent requires a cwd change, but headless cannot change cwd at runtime (`/cd`). An interactive pool could do it with `/cd`, but that costs memory and is a non-main path, so it was not adopted.
- To prevent re-litigation (already weighed — do not raise again):
  - **session-id is not a blocker.** Boot a pool slot in advance with an engram-controlled sid → when assigned, *adopt* that sid as the agent's identity, which keeps sid control (ADR-0008). Only "relabeling the sid of a running process" is impossible.
  - **Runtime cwd change is possible only through interactive `/cd`.** Not in headless. (Unverified: whether `/cd` *fully* retargets the file tools' cwd + reloads the new folder's CLAUDE.md — measure only if we seriously go with an interactive pool.)
  - **Windows has no `fork()`/COW** → a prefork (Zygote) that reuses a warmed V8 heap is impossible. Node does not expose interpreter fork either.
  - **Reusing a terminal (cmd `/c`) is pointless** — the cost is claude.exe init (Node/V8 + bundle + autoload), not the shell spawn (a few ms).
  - **A per-folder pool** (boot in advance for each active folder instead of retargeting) is possible even with headless + needs no `/cd`. But it requires the premise that "working folders are predictable and few."
  - **Memory vs speed = the two ends of the same dial.** A warm pool = pay memory cost to gain speed / idle kill = sacrifice speed to gain memory. With claude.exe, both cannot be maxed.
- Effective levers for fast spawn (instead of pooling): `--bare`(less boot via removing autoload) + a warm tier (do not kill hot agents). The fundamental way out = **API transport** (does not use claude.exe → agent = in-process task → the memory/speed dial itself disappears, §2 · candidate ADRs).
- 관련: §1 ③ (headless limits) · §2 (fleet hosting) · §4 candidate ADRs.

## Sources
- code.claude.com/docs: headless · cli-reference · sessions · permission-modes · hooks-guide · settings · agent-sdk/overview
- Agent SDK subprocess behavior: buildwithaws.substack.com/p/inside-the-claude-agent-sdk-from
- Ink/memory: medium.com/@sujaypawar/how-claude-code-actually-works · sathwick.xyz/blog/claude-code.html
- tmux vs Zellij 100-pane: tildalice.io/tmux-vs-zellij-100-pane-benchmark · github.com/zellij-org/zellij/issues/3594
- rate limit: platform.claude.com/docs/en/api/service-tiers
