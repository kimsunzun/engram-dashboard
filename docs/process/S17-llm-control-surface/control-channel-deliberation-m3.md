# S17 control channel — pivot from engram-ctl to in-band output marker (M3) (issue summary)

> **⚠️ The M3 conclusion of this document was later reversed.** Final decision = [ADR-0086](../../decisions/0086-제어-채널-듀얼-typed-입구mcpcli-sqlite-메일박스-first-마커m3-폐기.md) · research details = [consolidated research report](./control-channel-research-consolidated.md).

> **Status: direction settled = M3 · formally recorded = ADR-0085** (2026-07-15 — ADR-0080 discarded entirely, ADR-0014 is unrelated and left untouched). **The PRD/TRD §6 update and the Unit redesign are not done yet** (additional work). This document = a snapshot of the issues, options and measurements that led to the decision — the source of truth for the decision is now **ADR-0085**.
> Date: 2026-07-15 · anchors: ADR-0014 (CLI-via-Bash) · ADR-0080 (control surface) · ADR-0002 (capability matrix) · ADR-0079 (resume seed boundary) · ADR-0003 (core isolation).

## 1. Problem
The **channel** for agent A→B message sending (and for future UI/observation control). Original plan = **engram-ctl** (a Rust CLI called through Bash → WS → daemon, token auth; ADR-0014/0080, TRD §6 slice 1). The user raised: **in-band is far better for security and speed** — no token/port is given to the agent, so the attack surface shrinks, and there is no per-call process spawn + WS handshake, which is decisive for high-frequency messaging.

## 2. Options considered (M1~M5)
- **M1 engram-ctl CLI** (original): exposes the token in the agent env + a process spawn and WS handshake on every call. A loss at high frequency.
- **M2 MCP** (the daemon exposes a local MCP endpoint): best for structure and bidirectionality. But **the tool schema stays resident in the agent context (~0.5~1.5k tokens/agent)** + endpoint auth and permission gate. The direction ADR-0080 already rejected as "too much ceremony".
- **M3 output marker**: the agent puts a marker in its output text → **the daemon picks it up from the stdout it already owns** → the daemon delivers it like mail. Zero tokens, zero spawns, most direct.
- **M4 Bash sentinel + hook**: a hook intercepts the tool-execution path. Structured capture, independent of output format. But it is **not "we get the callback"** — it is an **indirect** method where a script launched by claude relays outward, + **the daemon has no ingress** (WS only) → one must be added + a tool-call cost per message.
- **M5 file drop**: robust, but a process spawn on every call.

## 3. Measurements (claude 2.1.170 · `.spike-control/` — throwaway, not tracked by git)
- **M4 hook fires ✓**: the PreToolUse hook fires in our exact headless invocation (`claude -p --input-format stream-json --output-format stream-json --verbose`). `tool_input.command` + `session_id` captured. Per-agent injection through `--settings <path>` and the cwd `.claude/settings.json` works. deny really blocks execution.
- **M3 marker emission near-perfect ✓**: with a strict single-line instruction the marker lines were **38/38 perfect** (valid JSON, recipient/body, escaping correct). The only trap = when the format is *described*, in 4/5 cases the marker was put inside a ```펜스 예시``` → solved by **the parser skipping lines inside code fences**. (A fenced-block marker cannot solve this problem and is actually worse → **single-line sentinel** adopted.)
- **Live/replay boundary confirmed in code**: the resume seed is loaded by `output_core.seed()` **without fanout**, and only live goes through `emit()` → **the marker parser never sees replay/scrollback** (no side effects re-run). Consistent with ADR-0079.

## 4. Decision (direction)
- **M3 = main path** (Claude Code CLI backend). Reason: with the CLI the daemon **already owns** the output stream, so picking up markers is the shortest path. A hook is an indirect relay rather than an in-process callback → heavy and awkward. Operation is **always json mode + high frequency**, so the **zero overhead** of M3 is decisive.
- **A real in-process callback = only on the API/SDK backend** (we own the inference loop → we receive tool_use directly). capability matrix (ADR-0002): the same ability "the agent emits a control signal", implemented per backend — **API = direct callback · CLI = M3**.
- **M4 = documented fallback** (a hedge against terminal mode / json format breakdown). Not implemented now — start when needed.
- **engram-ctl (ADR-0014/0080) direction = discard** — the formal supersede + PRD/TRD update + Unit redesign come after further discussion.

## 5. Rejected alternatives + reasons
- **engram-ctl (M1)**: token exposure + per-call spawn/WS cost (a loss at high frequency).
- **MCP (M2)**: resident context-token cost + endpoint/permission ceremony (the existing ADR-0080 rejection stands).
- **hook-as-primary (M4)**: indirect relay (not a callback) + a new daemon ingress + tool-call cost → backup only.
- **file drop (M5)**: a spawn on every call.

## 6. Open items (further discussion)
- **Formal ADR recording** — supersede engram-ctl (ADR-0014/0080) + the M3 decision.
- **Update the PRD/TRD (§6 engram-ctl table)**, redesign Units #1~4 (drop the engram-ctl premise).
- **★Parallel-session conflict★**: the working tree holds an **engram-ctl crate** made by a parallel session (`crates/engram-dashboard-ctl/`: catalog/envelope/error/v1.rs + a Cargo.toml member added, **uncommitted**) — the very thing we decided to discard. **Coordination needed** (discard/keep is the user's decision).
- **M3 detailed design**: marker protocol (sentinel, JSON schema) · fence-skipping parser · `ControlSignal` OutputEvent · suppressing the marker in display · multiple messages in one turn · daemon mail (scope of storage/delivery/ACK).
- Whether to run a Codex neutral design review.

## 7. Implementation seams (measured mapping — reference, not final)
- **Common delivery**: `AgentManager::write_stdin(agent_id, data)` (`manager.rs:638`) → `session.write_input` → target stdin. Daemon call site `connection_core.rs:714`.
- **M3**: add `OutputEvent::ControlSignal{target, message}` (`types.rs`) + decoder marker detection (`backend/claude.rs` — live TextDelta) + the daemon intercepts ControlSignal and calls `write_stdin` (`ws.rs`/`connection_core.rs`). Core isolation (ADR-0003) kept — marker detection only emits an event from core, routing is the daemon's.
