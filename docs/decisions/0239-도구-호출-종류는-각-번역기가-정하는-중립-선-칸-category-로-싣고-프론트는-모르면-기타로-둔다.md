# ADR-0239: Tool-call kind is carried in a neutral wire field category that each translator decides, and the frontend falls back to Other when it does not know

- 상태: 확정 (2026-09-27, basis: user decision U5 (TRD `docs/process/S21-chat-ux/trd.md` §1) + TRD 5th edition §4-1 · §4-2 · §4-3 + TRD 5th-edition review FIX applied (TRD §12) · before implementation · code unchanged) · 부분 폐기 by ADR-0263 (decision 4's auto-expand of the last group while running)
- 관련: TRD `docs/process/S21-chat-ux/trd.md`(§1 U5 · §4-1 · §4-2 · §4-3 · §4-6 · §6 · §7 B4 · FE-2 · I1 · §9 · §10 0239) · research `docs/research/chat-ux-four-features-2026-09-27.md`(§3) · ADR-0241(tool-end result — the 「오류 N」 and 「거부 N」 of the same group) · ADR-0004(backend knowledge isolation) · ADR-0051(row kind ↔ rail) · ADR-0050(StructuredTextView is a pure render) · ADR-0167(a window command without `help`) · step-log S21 · Amended by ADR-0263 (decision 4's auto-expand of the last group while running)

## 맥락
Of the four chat-screen items, tool-call grouping folds consecutive tool rows (≥2) into one group and puts on its head a summary line that joins the per-kind counts with ` · ` in a fixed order (TRD §4-3). For the summary to count kinds, every call needs a kind. Tool names and item types differ by vendor — claude carries tool names like `Read` · `Grep` · `Bash`, and codex carries item types like `commandExecution` · `fileChange` and `commandActions`. Today the icon of a single tool row is chosen by a name heuristic in the frontend (`StructuredTextView.tsx:150-167`).

This ADR records **who** decides the kind and **where** the group is made. The wire shape of the end result (error · refusal) is on a different axis, so we split it off into ADR-0241 — if the two were mixed in one ADR, reversing one side would spread the discarded stamp to the other, and the next session could not tell which alternative belongs to which decision (TRD §10).

## 결정
Notation — **[사용자]** = a decision the user answered · **[고름]** = an internal implementation the TRD picked.

1. **We carry the neutral kind `ToolCategory { Read, Search, List, Edit, Command, Web, Agent, Mcp, Other }` on the wire.** Each backend translator decides it, and vendor tool names and item types do not come onto the wire (ADR-0004).
   - agent domain (`crates/engram-dashboard-agent/src/types.rs`): `category: ToolCategory` on `OutputEvent::ToolCall` — not `Option` (the translator always picks one).
   - wire (`crates/engram-dashboard-protocol/src/messages.rs`): an enum of the same nine variants · `category: Option<ToolCategory>` on `StructuredEvent::ToolCall` (`#[serde(default, skip_serializing_if = "Option::is_none")]` · `#[ts(optional)]`). The daemon conversion (`connection_core.rs:777-789`) is a one-to-one match.
   - TS touchpoints = the generated `ToolCategory.ts` (`"Read" | "Search" | "List" | "Edit" | "Command" | "Web" | "Agent" | "Mcp" | "Other"`) · `category?: ToolCategory` on the ToolCall in `StructuredEvent.ts`.
2. **The decision rule lives inside the translator.**
   - claude (the `tool_use` branch of `consume_block`) — a tool-name table `fn tool_category(name) -> ToolCategory`: `Read` ← `Read` · `NotebookRead` / `Search` ← `Grep` · `Glob` / `List` ← `LS` / `Edit` ← `Edit` · `MultiEdit` · `Write` · `NotebookEdit` / `Command` ← `Bash` · `PowerShell` · `BashOutput` · `KillShell` · `KillBash` / `Web` ← `WebFetch` · `WebSearch` / `Agent` ← `Task` · `Agent` / `Mcp` ← starts with `mcp__` / the rest (`TodoWrite` etc.) `Other`. The table is an exact match (case-sensitive), so a new tool it does not know falls to `Other`. [고름] `Glob` as `Search` — Claude Code itself groups it as 「파일을 찾는다」.
   - codex (`tool_call` in `codex/decoder.rs`) — `commandExecution` by `commandActions[].type`: all `read` = `Read` · all `listFiles` = `List` · there is a `search` and no `unknown` = `Search` · only `read` and `listFiles` mixed = `Read` · even one `unknown`, or the array is missing or empty = `Command`. `fileChange` = `Edit` · `webSearch` = `Web` · `mcpToolCall` = `Mcp` · `collabAgentToolCall` = `Agent` · `dynamicToolCall` = `Other`.
   - The kind is decided first from the **whole item** that `tool_call` receives, and only `args_json` passes the size cap (`bounded_args_json`) — a kind is never lost because of the cap's truncation. History restore (`ItemOrigin::History`) also passes through the same function.
3. **The frontend treats what it does not know as 「기타」.** The `tool` entry of the accumulator (`structuredAccumulator.ts`) gets `category: ToolCategory` · a defensive read `normalizeToolCategory` — if it is one of the nine known words, as is; if missing or unknown, `'Other'`. It holds before the generated file arrives · with an old daemon · with a newer daemon. [고름] The icon of a single tool row is the kind icon if `category !== 'Other'`, otherwise today's name heuristic — with an old daemon, today's look as it is.
4. **The frontend makes the group as a pure function at render time — neither the accumulator nor the backend makes groups.** `groupToolRuns(items, turnOpen)` (new `src/components/slot/chat/toolRuns.ts`):
   - Candidates = from the first `tool` row to the last `tool` row. What can sit between = rows that are not drawn (`rowKindOf` = `skip`) and **non-empty thoughts** — absorbed [사용자 U5]. Any other **drawn** row (text · user bubble · divider · outcome · error · unknown event · any other `structured`) breaks it. It is a group only when there are ≥2 `tool` rows, and the thought/skip rows after the last `tool` are not members.
   - `live` = `turnOpen` and everything after the group is only skip rows and non-empty thoughts — we do not fold just because a thought came after (the flicker of it expanding again when tools continue). The effective expansion = the user toggle `?? live` — the user toggle beats the auto-fold (the expansion state is in the in-memory store `toolGroupStore`).
   - Key = the backend id of the first call (`tool:<id>`) · if none, `item:<itemId>` — the accumulator rebuilds the same event sequence with the same `itemId`, so it is the same after a replay.
   - `StructuredTextView` stays a pure render — it adds no state, effect or store subscription, and the new child `ToolGroupRow` carries the expansion read and toggle. The rail position is computed from the **row list** (a group = one `'assistant'` row · no `ChatRow` rail inside a group — ADR-0051).
   - LLM path = the command `chat.toolGroup.setExpanded { slotId, groupKey, expanded }` — no `help` (frontend state that exists separately per window · ADR-0167).
5. **We do not raise `PROTOCOL_VERSION`** — an old daemon → there is no field → the frontend `Other`. An old shell → it ignores the unknown field (the shell does not interpret tag1 JSON, it carries it — `rg StructuredEvent src-tauri/src` 0 lines). The same judgment as `TurnEnd` (the comment at `messages.rs:743-763`) (daemon→shell one direction · the old side has nothing to misread).

## 거부한 대안
- **A tool-name table in the frontend** — vendor knowledge leaks into the frontend (ADR-0004 「백엔드 확장」 — vendor knowledge in one place, `backend`).
- **The backend makes the group** — it puts a display concern on the wire (TRD §10). The TRD gave no other reason.
- **A non-empty thought between tools breaks the group** [사용자 U5 — chose absorption] — if it breaks, groups hardly form in a claude turn where a thought comes between every tool. The price of absorption = you have to expand to see the thought (codex TUI precedent = absorption). With this alternative, one line of the absorption decision in `groupToolRuns` changes to 「끊는다」 (breaks).

## 근거
- **User decision U5 (2026-09-27)** — the recommended option as is (TRD §1).
- **Classification reading** — the vocabulary of codex `commandActions[].type` (`read` · `listFiles` · `search` · `unknown`) is a reading (TRD §4-2). The `unknown` → `Command` of fixture `tool_end_m7.jsonl` is included in the tests (TRD §4-6).
- **Review** — TRD editions 1–5 `/review trd` rounds (TRD §12).
- ★**Verification status**★ — before implementation (TRD 5th edition · code unchanged). Test plan = TRD §4-6 (grouping-rule table · summary · rail · normalization · claude name table · codex combination table · an item over the cap · serialization round trip).

## 영향 / 불변식
- **All construction sites and exhaustive patterns in one commit** (the compiler points to them — TRD §4-1 list · §7 B4). The ring weight (`estimate_cost_bytes`) is 0 because it is a fixed-size enum.
- **Generated files** — `crates/engram-dashboard-protocol/bindings/ToolCategory.ts` · `StructuredEvent.ts` are baked by `cargo test -p engram-dashboard-protocol` and the CI sync gate is their owner — we do not write them by hand. For a while the frontend uses a local literal union of the same nine words, and after B4 · B5 · FE-2 are all committed, one commit (I1) switches it to the generated import (TRD §7).
- **Vendor tool-name drift** — if claude renames a tool, it falls to `Other`. The group still forms and only the summary text becomes 「기타」 (TRD §9).
- **The expansion state is in memory** — it resets on a webview refresh (the same level as the layout — CLAUDE.md 「LLM-우선 제어」). Re-subscribe and replay do not erase it.
  > Note 2026-10-04 (decision unchanged): 「레이아웃과 같은 수준」 is no longer right — the layout has been persisted in `shell\state\state.json` since storage management P3b3 (`f9db35a`). The expansion is still in memory and is not kept (TRD `docs/process/S21-storage/trd.md` §6-0 「남기지 않는 것」 — user decision 2026-10-02).
- **Row kind ↔ rail (ADR-0051)** — the rail is computed from the row list and no rail row is placed inside a group. If violated, the rail computation and the DOM diverge in one body.
- `OutputChunk::ToolCall` (`messages.rs:907` — an S14 snapshot remnant) is not touched.
