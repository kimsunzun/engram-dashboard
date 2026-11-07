# ADR-0079: Restoring the conversation scrollback on JSON (RichSlot) mode resume — the daemon reads the Claude `.jsonl` and seeds the OutputCore buffer (single source · before the pump)

- 상태: 확정 (2026-07-13, basis: /research OSS survey + Codex adversarial review + live CLI measurement)
- 관련: ADR-0044 (json mode wiring, stream-json decoder) · ADR-0008 (session restore, controlled sid) · ADR-0007 (epoch resubscribe) · ADR-0046 (view-scoped replay, gen fence) · ADR-0049 (thinking) · ADR-0052 (user echo uuid dedup) · CLAUDE.md §5(LLM-우선 제어)·§1(OutputSink) · `backend/claude.rs` (decoder, build_spec) · `agent/output_core.rs` (Ring/replay) · `agent/manager.rs` (spawn_session) · step-log S(복원 UX)

## 맥락
When a session is resumed in JSON (RichSlot) mode, the chat window comes up as a blank screen (new turns only). Two layers of cause (live measurement 2026-07-13, claude 2.1.170):

1. **claude does not re-emit the past.** `claude --resume <sid>` (stream-json headless) restores only the model context and does not send past turns to stdout again. There is also **no** CLI flag that re-emits past history to stdout (`--replay-user-messages` echoes only new stdin input; `--resume`/`--continue`/`--fork-session`/`--from-pr` only inject context). So it is not that we missed a flag.
2. **The replay buffer is per process.** The Ring (replay buffer) of `OutputCore` holds only what the current process emitted and is reset on respawn/restart → the resume process starts with an empty buffer.

In terminal (xterm) mode the TUI repaints the past conversation through the PTY and it shows up in xterm, but JSON mode has no repaint, so this asymmetry arises. Since resume = turning the process off and on, the past must be loaded again at every resume (including daemon restart and re-activation).

## 결정
**On a resume spawn, the daemon reads the Claude `.jsonl` transcript, converts it into `OutputEvent`s, and seeds the `OutputCore` replay buffer — before the pump (live output) starts.** Live events are then appended to the same buffer. The buffer becomes the single source and is delivered to the frontend through the existing replay path (subscribe→replay→live), and RichSlot renders it identically to live.

- **Single source = the `OutputCore` buffer.** The source is the buffer, not the `.jsonl` file itself. The past is filled by seeding only, and live by the pump only, and the two do not overlap (because resume does not re-emit).
- **Reuse.** The past-turn mapping uses the block mapping of the existing stream-json decoder as is (assistant text→`TextDelta`, tool_use→`ToolCall`, user/tool_result→`Structured{kind:"user"}`, thinking→`Structured{kind:"thinking"}`). No new `OutputEvent` variant — the frontend accumulator and RichSlot already render these events, so frontend changes are minimal.
- **Isolating claude-specific knowledge (ADR-0004).** The `.jsonl` path (cwd→project slug), the line type filter, and the parsing live only in `backend/claude.rs`.
- **resume = full reload.** The buffer is a reconstructible cache, and the `.jsonl` on disk is the durable truth. Re-seed on every resume, including daemon restart and re-activation.
- **Scope and defaults.** JSON (RichSlot) mode only (terminal unchanged). Sub-agent (sidechain) turns and summary lines are skipped (original turns only). For large files, tail-parse from the end of the file up to the Ring limit (2MB/4096), and accept truncating from the oldest when over the limit (reusing the existing `Truncated` report). No past/present divider (a continuous stream). Lazy (loading older on scroll up) is a follow-up task.

## 거부한 대안
- **The frontend reads the `.jsonl` directly and renders** — violates §5 (the frontend acquires a capability the backend brain cannot hold + the pure-I/O principle is broken), and the live/restore render paths become dual. If the daemon reads and delivers, the LLM can control it through the same path.
- **Seed-then-append through a separate History channel (the frontend stitches two sources, past + live)** — in the reference measurement this approach (Claudia) has a boundary-turn double-render race bug. If the source is merged into one buffer (the Crystal way), that race disappears structurally and no dedup code is needed.
- **Create our own durable event journal now (B2)** — excessive at the current claude-only stage (the cost of schema normalization, WAL, crash consistency). Unnecessary since the `.jsonl` is already a durable source on disk. Reconsider together with per-backend mappers when generalizing to codex/gemini (principle 0: high-cost, uncertain things then).
- **Replace even the live source with `tail -f` of the `.jsonl`** — the `.jsonl` is in units of completed messages (no streaming deltas), so live token streaming (typing) disappears, and the everyday hot path becomes coupled to the claude file format (backend-agnostic destroyed). The `.jsonl` is only the source for seeding the past, and live keeps the existing stream-json path.
- **Expecting a flag that turns on history re-emission for resume** — there is no such CLI flag (confirmed by measurement).

## 근거
- **/research OSS survey (design-decision mode):** the standard of the Claude Code CLI wrapper class (Claudia/opcode = our stack, claude-code-webui, Crystal) = on resume, read the `.jsonl` directly and reconstruct. The only peer with a server-side store (Crystal) removes boundary duplication structurally through a **single source** (record live into the store + re-query). Seed-then-append (Claudia) has a race bug at that point (source checked directly).
- **Codex adversarial review:** pointed out explicit history frames, event-sourcing, the §5 control surface, and seq identity → absorbed by a single buffer + (if needed) markers. The principle "do not disguise restoration as live" is satisfied by the seed reproducing the past turns as they are (not as live deltas).
- **Live CLI measurement (2026-07-13):** `claude --resume` (stream-json) does not re-emit past turns to stdout, and no re-emit flag exists. `--replay-user-messages` (which we already pass) is only for echoing new input.

## 영향 / 불변식
- **seed-before-pump (load-bearing):** on a resume spawn, the `.jsonl` seed must complete **before** `session.start_pump()` (manager.rs). If the pump is turned on first, a fast first live event arrives ahead of the seed and the order breaks. This order is the nail of consistency.
- **Single source:** the past is filled by seed and live by the pump only, and they do not overlap (premise: resume does not re-emit). When the buffer is empty after a restart, seed the `.jsonl` again (cache reconstruction).
- **Isolating claude knowledge (ADR-0004):** the `.jsonl` parsing, slug encoding, and line type knowledge live only in `backend/claude.rs`. Transport and core do not know.
- **Frontend = pure I/O (§5):** the frontend does not read the file. The daemon reads it and delivers it through the existing event path.
- **Tests:** the `.jsonl` parser is a pure function with no external dependencies → unit-test with real fixtures (`backend/fixtures/`) (ADR-0012 seam isolation).
