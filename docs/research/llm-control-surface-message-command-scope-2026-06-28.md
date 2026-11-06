# LLM control surface — message/command system: process & research scope (draft)

**Status:** draft plan — awaiting owner approval. A scope document before research starts (prep for the preliminary survey).
**Written by:** dashboard1 (worktree `engram-dashboard-a1`, branch wip/a1). Written in answer to the owner's "프로세스 짜봐" (draw up a process) request while the owner was away.
**Next action:** owner returns → reviews this scope → decides research intensity (`/research`) → option set → PRD decision.

## 1. Target scenario (the first milestone the owner described)

The LLM must be able to do the following **by command** (the human UI is secondary):
1. Launch the first screen (main window).
2. Left = agent list, right = 4-quadrant split.
3. Create one more popup → inside it, a 2-way split.
4. Spawn agents 1·2·3·4 in a specific folder.
5. Place agents 1 and 3 in quadrants 1 and 4 respectively.
6. Confirm communication between agents.

## 2. Essence

This is the first implementation of CLAUDE.md §5 **"LLM 제어 표면"** — gather the backend + all of the UI/layout into a **single intent command bus** that the LLM calls, with human clicks as a secondary input that shakes the same handle. Three pieces:
- **Command bus** — the LLM calls layout split · slot placement · popup · agent spawn/placement.
- **Message system** — communication between agents (scenario step 6).
- **Layout control** — quadrants/popup/split (scenario steps 2·3·5).

## 3. S14 boundary (★ coordination point — owner decision needed)

The target scenario's **layout (split · popup · slot placement) overlaps the S14 ViewManager area that dashboard2 is working on now** (ADR-0035/0036/0037). Principle:
- The command bus is a layer that **drives** the ViewManager, **not a reinvention.**
- In research · design, read the S14 TRD · ADRs first, and draw a clear line between "what the ViewManager already exposes ↔ what the command bus newly layers on".
- This boundary is a point the **owner decides by coordination** so that the two workstreams (dashboard1 · 2) do not collide.

## 4. Development step process (applying the CLAUDE.md macro flow)

1. **Research (optional)** — `/research` design-decision mode. OSS survey + option set. (This document is that scope.)
2. **PRD — owner decision** — the owner chooses among the options (no arbitrary adoption). Adversarial verification with `/review prd`.
3. **TRD** — fix the interface · command schema. Implementation forks go to the owner for decision again. Record ADRs immediately.
4. **Module boundary (DDD)** — the command bus seam (LLM call ↔ store action ↔ ViewManager). Make it verifiable in isolation.
5. **Implementation + TDD** — coder/reviewer (`/review code`)/QA (`/qa`) subagents. Main does the orchestration.

## 5. Research scope (3 axes for the next step)

1. **Command bus / control-surface patterns** — how mature OSS solves a single intent bus that humans and LLMs call through the same handle (command pattern · intent registry · event sourcing etc.).
2. **Inter-agent communication / orchestration** — revisit the ADR-0014 candidates (Erlang OTP/Ractor · Temporal/Restate · LangGraph/MS Agent Framework · A2A) → compare under engram's constraints (Rust · swappability · LLM control).
3. **S14 ViewManager interface** — read the ViewManager's current API, and see how to layer layout commands (split · popup · placement) on top of it.
- Horizontal axis of the output: **break each of the 6 target-scenario steps down into command primitives** → derive which handles are needed.

## 6. Output · decision authority

The research output = an **option set + trade-offs + missed alternatives** report (`docs/research/`). **Adoption is the owner's decision** (no arbitrary fixing). Major decisions go into ADRs.

## 7. Multi-agent coordination memo

Shared repo, with one worktree split applied: dashboard2 = `engram-dashboard` (master, S14/ViewManager), dashboard1 = `engram-dashboard-a1` (wip/a1, this work). Since the layout commands interface with S14, **the owner must coordinate the boundary and merge timing of the two workstreams** (§3).
