# S10 — Backend abstraction design (AgentTransport / OutputEvent)

> **✅ Implementation complete (2026-06-12, commits `60fe859`~`fb50917`).** All 9 steps + fable gates (2/3/6/final) GO. Zero regressions (test 38 / headless·smoke PASS). Concrete signatures are in `impl-spec.md`, the timeline in `../step-log.md` S10. Console (PtyTransport) implemented and verified; ApiTransport and codex/gemini are stubs (to be filled in later).

Basis: converged GPT+Gemini composition review (`web 검토 2026-06-12`) + user decisions.
Scope: a unified interface for multiple backends (claude/codex/gemini console + API). **For long-term, low-risk items, abstract thoroughly (over-engineering allowed); API is a stub shell only; the three consoles are implemented.**

## Decisions (user)
- Since this is a long-term project, lay down the risk-free abstractions thoroughly now.
- **The API transport is a shell (stub) only** — satisfy the interface only, internals later (pay-per-use API ~a month later).
- **Build the three consoles (claude / codex / gemini CLI)** — all terminal-based.

## Converged feedback reflected (GPT+Gemini agree)
1. **Raise the seam from `emit(bytes)` to `emit(OutputEvent)`.** xterm is not the single source of truth; it is one of the projections.
2. Replay of raw bytes is fragile (truncated ANSI/partial messages) → **the console keeps a terminal-bytes ring (known limitation)**; the semantic event log / dual storage / projection are **filled in when the API arrives**.
3. **No explosion of capability bools** — per-area descriptors (input/output/control/session/model).
4. If input is bytes too, it leaks → the **`InputEvent`** concept (PTY=Raw, API=Message).
5. Missing control verb: add **interrupt (≠kill)** now; phase in reconfigure/snapshot/graceful-shutdown.

## Layers (target structure)

```
AgentManager
  └ HashMap<AgentId, AgentSession>            수명 + 복원 오케스트레이션
       │
   AgentSession  (에이전트당 1, 상태 독립)
     ├ OutputCore  : seq · replay · fanout   ← OutputEvent 단위로 동작(공용 코드 1벌)
     ├ capabilities: Capabilities (영역별)
     └ transport   : Box<dyn AgentTransport>
                          │
   trait AgentTransport  (seam)
     데이터: send_input(InputEvent) / (emit OutputEvent → OutputCore)
     제어  : start · interrupt · kill · resize        [지금]
             reconfigure · snapshot · graceful_shutdown [단계화/후일]
     질의  : capabilities()
        ├ PtyTransport  : master/slave·pump스레드·ConPTY·kill6·JobObject  (콘솔 공용)
        └ ApiTransport  : 껍데기(unimplemented), HTTP 스트림은 나중
```

## Core types (conceptual — signatures are finalized at implementation)

```
enum OutputEvent {
    TerminalBytes(Vec<u8>),         // 콘솔(claude/codex/gemini) — 지금 쓰는 유일 variant
    // ── API 등장 때 채움(지금은 미정의 또는 정의만) ──
    // TextDelta(String) · MessageDone · Usage{..} · ToolCall{..} · Error{..}
}

enum InputEvent {
    Raw(Vec<u8>),                   // PTY: 키 입력 바이트
    // Message(String) · Cancel · Reconfigure{..}   (API용, 나중)
}

struct Capabilities {               // bool 나열이 아니라 영역별 묶음
    input:   { raw, message, attachment }
    output:  { terminal_bytes, markdown, tool_events, usage }
    control: { resize, interrupt, cancel, graceful_shutdown }
    session: { resume, snapshot, cwd_env }
    model:   { select, temperature, max_tokens }
}
```

### Console backends (AgentBackend — held by PtyTransport as a parameter)
- `ClaudeBackend`  : `--session-id`/`--resume` + `sessions/<pid>.json` tracking. **Implemented and verified (S9)**.
- `CodexBackend`   : build_command/tracking — **flags finalized by a spike on the CLI subscription day (a few days later)**. Until then best-guess + marker.
- `GeminiBackend`  : same — finalized after a spike on the gemini CLI flags.
- All three share `PtyTransport`; the differences are only args and session tracking.

### ApiTransport (stub shell)
- Implements `AgentTransport`, but its methods return `Unsupported`/`unimplemented!`. Capabilities are all reported as false/unsupported. The HTTP stream and event conversion are filled in on the day an API model is attached.

## Phase split (now vs later)
| Item | Now | Later |
|---|---|---|
| OutputEvent seam | ✅ enum + TerminalBytes | TextDelta/Usage/ToolCall variants |
| InputEvent | ✅ enum + Raw | Message/Cancel/Reconfigure |
| Capabilities per area | ✅ structure + console values | fill in API values |
| control: interrupt | ✅ (Ctrl+C / immediately useful for consoles) | — |
| control: reconfigure/snapshot | definition only | implement for the API |
| replay | console terminal-bytes ring (current) | semantic event log + display/event separation + projection |
| Console backends | claude✅ / codex·gemini structure+stub | on the day the CLI is attached, finalize and verify flags by spike |
| ApiTransport | stub shell | internal implementation |

## Current S9 code → target migration
- Decompose `PtySession` → `PtyTransport` (master/slave·pump·kill6·job) + `OutputCore` (hoist seq/replay/fanout into AgentSession).
- The `pump` thread (the read part of the current drain) emits `OutputEvent::TerminalBytes`, and OutputCore post-processes.
- `PtyManager` → `AgentManager` (holds `Arc<dyn AgentSession>`). restore/fallback is generic, based on `capabilities().session.resume`.
- `claude.rs` → `ClaudeBackend`. Add `CodexBackend`/`GeminiBackend` (stubs). Add the `ApiTransport` stub shell.
- write_stdin → `send_input(InputEvent::Raw)`. Separate interrupt from kill.

## Open / review points (candidates for ③ detailed web questions)
- pump→OutputCore handoff: channel vs callback (Arc OutputCore)? Performance and ownership.
- How far to define the OutputEvent enum now (unused variants up front vs add later).
- Whether to merge the AgentBackend (console args) trait into PtyTransport or keep it separate.
- PTY implementation of interrupt (inject Ctrl+C=0x03 vs signal).
- Capability as trait methods vs data descriptor vs Extension Trait (`as_interactive()`).

## Consolidated three-way high-level review (GPT + Gemini + fable, 2026-06-12)

**Converged (all three agree):** seam from the event viewpoint · guard against capability bool explosion · missing interrupt/reconfigure verbs.

**★Found by fable alone (grounded in real code — key reinforcement)★:** the leak is not the input (emit bytes) but the **shutdown boundary**.
- The kill 6 steps (manager.rs:442-503) mix transport-private resources (child/job/master) with synchronizing the shared OutputCore's pump shutdown, so they cannot be put wholesale into `transport.kill()`.
- The exit-code decision in `transition()` (drain.rs:110-140) differs per transport (PTY=exit code / API=stream closed·cancelled·error).
- **→ Decomposition:** `transport.shutdown()` (force-terminate resources, idempotent, only the child→job→master order is transport-private) + `core.join_pump(timeout)` (synchronously wait for pump shutdown, shared) + `TerminalReason` (Exited{code}/Killed/StreamClosed/Error — produced by the transport; the idempotent status-transition rule is shared).
- **kill = the composition of `shutdown()` + `join_pump()`.** This decomposition also naturally removes the duplication between `remove_session` and `kill_agent` (manager.rs:357-387 vs 442-503) → evidence that the dividing line is right.
- Shutdown flag ownership must be stated (transport sets / pump shutdown part reads).

**Capability decision (fable, at the current scale):** a few bools are enough. Only session/resume gets a small descriptor (the mechanisms differ). Extension Trait / full negotiation is over-design (loses compile-time capability → conflicts with the principle "implement only on top of the abstraction"). Carry capabilities in the `AgentInfo` snapshot to unify the frontend coupling point.

**Control verbs settled:** `start` · `send_input(InputEvent)` · `resize` · **`interrupt`** (≠kill, PTY=inject `\x03` / API=HTTP cancel) · `shutdown` (resources) · **`reconfigure`** (PTY=respawn / API=parameter change) · `capabilities`. Pump shutdown is `core.join_pump()`.

## Low-level consolidation (fable + GPT + Gemini, 2026-06-12)

### Three-way convergence — settled
1. **TerminalReason**: flat enum, produced by the transport → the core maps it to `AgentStatus` (idempotent + **finalize exactly once**: OnceCell/compare_exchange). Variants: `Exited{code:Option<i32>}` · `Killed` · `Interrupted` · `StreamClosed` · `Cancelled` · `Error(String)`. Note: **do not expose raw lib errors (reqwest/nix) directly** → map to domain strings (Gemini). `StreamClosed`≠success (GPT — leave room for an auxiliary last_error).
2. **Two explicit verbs shutdown()/join_pump()** (no RAII — there is no async Drop [Gemini] + the master drop timing cannot be pinned down [fable]). Order: `transport.shutdown()` (master drop/cancel token → the pump read wakes with EOF/Err and exits) → `core.join_pump(timeout)` → status finalize once. Drop is only a safety net. **Ownership**: transport = master/writer/child/shutdown flag/job, core = subscribers/replay/seq/status/drain_handle/drain_done_rx. The transport raises the Killed decision as a reason, so the **shared atomic flag is removed**. Causality is guaranteed by resource closing, not by timing (Gemini).
3. **InputEvent**: `Raw(bytes)` only for now. **Cancel is not a variant** → control verb `interrupt()` (all three agree).
4. **args isolation**: PtyTransport does **not know** claude/codex/gemini. A pure `CommandSpec{program,args,env,cwd}` is injected. claude/codex/gemini each produce a CommandSpec (a thin backend layer). No dyn/generic over-abstraction (Gemini). Keep SpawnMode (Fresh/Resume) neutral; only the flag-name mapping lives inside the backend.
5. **Final control verbs**: start·send_input·resize·interrupt·shutdown·reconfigure·capabilities. kill = shutdown+join_pump composition.

### ✅ Resolved — OutputEvent/InputEvent: turn them into an interface with an extensible enum and move on (user decision)
**Decision:** define `OutputEvent` (`TerminalBytes` only) and `InputEvent` (`Raw` only) as **extensible enums**, and keep the core variant-agnostic (`_ => ignore`). When the API arrives, adding a variant is one line (swappable = interfacing done). "Whether to write even the dead variant names now" is a zero-cost side issue, so left undecided — proceed without them. Principle: "if it only needs to be swapped later, make it an interface and move on."

## Implementation order (④ — build/test/commit at each step, fable gate at the end)

Since this restructures verified S9 code, the goal is **zero regressions**. After each step keep `cargo test --lib` + `cargo run --example headless` passing.

1. **Neutral types/enums** — `OutputEvent{TerminalBytes}` · `InputEvent{Raw}` (extensible enums) · `TerminalReason{Exited{code}/Killed/Interrupted/StreamClosed/Cancelled/Error(String)}` · `Capabilities` (per area, console values) · `CommandSpec{program,args,env,cwd}`. Neutralize `PtyChunk`→`OutputChunk`.
2. **Extract OutputCore** — a struct that separates seq/replay/subscribers/status out of PtySession. `emit(event)` · `finish(reason)` (idempotent + finalize once) · `join_pump(timeout)` · subscribe/unsubscribe. Unit tests.
3. **AgentTransport trait + PtyTransport** — move master/writer/child/shutdown/job + the pump thread into PtyTransport. `shutdown()` (guarantees master drop before returning) · the pump emits `OutputEvent` and on exit produces a `TerminalReason` → `core.finish`. headless passes.
4. **AgentBackend (produces CommandSpec)** — `ClaudeBackend` (current claude.rs) + `CodexBackend`/`GeminiBackend` stubs. Keep SpawnMode neutral; only the flag-name mapping in each backend.
5. **AgentSession** — holds OutputCore + transport. Exposes `write_input`/`resize`/`interrupt`/`kill` (=shutdown+join_pump)/`reconfigure`/`capabilities`.
6. **AgentManager** (PtyManager renamed) — holds `Arc<dyn AgentSession>`. spawn/restore/fallback made generic based on `capabilities().resume`. Port the S9 restore logic. Eliminate the `remove_session` duplication.
7. **ApiTransport stub shell** — implements AgentTransport but with `Unsupported`/`unimplemented!`. All capabilities false.
8. **Rewire commands/lib/frontend** — `send_input`/`interrupt` commands, include `capabilities` in the `AgentInfo` snapshot, TS mirror.
9. **fable gate** + headless + full tests.

**Keep the invariants (no regression):** kill's "master drop→reader EOF→join" causality (now as the two verbs shutdown→join_pump), the §10 lock rules, no lock held at drain send, epoch resubscribe, best-effort tracker.

## (Reference) The split at review time: define the OutputEvent API variants now?
- **fable + GPT (2): do not define up front.** It is a wire format (Serialize, shared with the frontend), so dead variants leak into TS + the API shape is unknown = speculative over-engineering. For now only `TerminalBytes` + neutralized names. Instead secure only the **core being variant-agnostic** (consumer `_ => ignore`) = an extension-safe structure.
- **Gemini (1): define up front (strong).** LLM primitives (TextDelta/Usage/ToolCall) are low-risk and quasi-standard. Stabilize the core fanout/replay match arms early → the API only plugs in the transport. `Extension(Value)` escape hatch.
- **Manager recommendation: do not define up front.** "Low-risk over-engineering" applies only to "things whose extension does not break and whose retrofit is expensive" — adding an OutputEvent variant later is cheap (consumer ignores it), so it does not clear that bar. But the genuine common ground of the three, **"make the core variant-agnostic and extension-safe"**, is secured now.
