# ADR-0062: AgentList MVP status display = 5-glyph vocabulary / mapping to the current backend's 3 states

- 상태: 확정 (2026-07-10, basis: pre-PRD user decision (5-glyph vocabulary) + the actual status enum confirmed by grounding) · Color decision partially amended by ADR-0075 (color allowed, e-ink neutralized) · 부분 폐기 by ADR-0168 (status display changed from characters to icons)
- 관련: CLAUDE.md §5 · ADR-0005 (status notification split) · ADR-0002/0030 (capability) · `crates/engram-dashboard-core/src/agent/*` (AgentStatus enum) · `src/components/agent/AgentList.tsx` (statusGlyph) · step-log "에이전트 트리·프리셋" · Amended by ADR-0168 (status display changed from characters to icons)

## 맥락

For the agent tree (AgentList) MVP, the user decided to show each row's status **by "shape" (glyph), not by color** (for e-ink contrast — status must stay distinguishable in black and white). The decided vocabulary has 5 states:

- `●` working · `◐` waiting for input · `○` idle · `◻` stopped · `✗` error

But when we checked the actual backend status enum by grounding, it is only `AgentStatus = Running | Exiting | Exited{code} | Failed{message} | Killed` — **there is no signal that separates "working vs waiting for input vs idle"** (all are lumped into `Running`, and PTY output activity and whether it waits for input are not tracked). So the 5-glyph vocabulary the user decided on cannot be mapped 1:1 onto the actual status.

## 결정

**Implement the 5-glyph vocabulary as is, as a pure mapping function (`statusGlyph`), but light up only the 3 that the current backend can distinguish.**

Current mapping:
- `Running` → `●` (working)
- `Exited` / `Killed` → `◻` (stopped)
- `Failed` → `✗` (error)
- `Exiting` → `◻` (transitioning to stopped — just before terminal)

`◐` (waiting for input) and `○` (idle) are **defined only as vocabulary and are not lit for now** — when the backend later emits an "output activity / waiting for input" signal, the mapping is filled in then. `statusGlyph` is a pure function with no external dependencies, so every branch is pinned by headless unit tests.

The existing invariant that the frontend decides terminal from the `agent-list-updated` list, not from `status_changed` (ADR-0005), stays as is — AgentList receives status through list updates.

## 거부한 대안

- **Add output-activity tracking now to separate working/idle/waiting for input** — splitting `Running` would require planting last-output-time and waiting-for-input detection in OutputRouter/core. That is **backend internals that have not been measured** (what counts as "waiting for input" differs per backend, and the decision differs between TUI and JSON mode), so under the CLAUDE.md §0 criterion "high-cost and uncertain → put in only a shell/definition and fill it in when measured" it falls here. Introducing it early in the MVP risks fixing a wrong heuristic in place. → Lay down only the vocabulary and leave it unlit.
- **Shrink the vocabulary to 3 to fit the actual enum** (only ●◻✗) — the user decided on 5 glyphs, and if the vocabulary is laid down as 5 in advance, only the mapping needs filling in when the backend signal appears (low risk, long horizon = over-engineering allowed, CLAUDE.md §0). Shrinking to 3 means rework to grow the vocabulary again later + reversing a user decision.
- **Show status by color** (the existing `AgentTree.tsx:33` statusColor approach) — on e-ink (black and white) the color is lost, so status cannot be told apart. The user explicitly decided "shape, not color". New UI is variable-only + glyph shape carries the status.

## 근거

- **User decision = 5-glyph vocabulary** (step-log, pre-PRD consult). The vocabulary is the user's domain; the mappable range is the backend's reality.
- **Grounding fact:** the actual `AgentStatus` has no working/waiting-for-input/idle distinction (Running only). We do not fill the gap by interpretation, and implement honestly as "light up 3 + 2 vocabulary entries left unlit".
- **Isolation as a pure function (ADR-0012):** `statusGlyph(status) → glyph` has 0 external dependencies → every branch is pinned by headless unit tests, and when the backend signal grows, only this function is edited.

## 영향 / 불변식

- **`statusGlyph` is a pure function + unit tests for every branch** — a mapping change touches only this function + its tests.
- **◐ and ○ being unlit is intentional, not a defect** — the next session must not reopen this mapping with "why don't waiting-for-input/idle show up". Filling in the mapping when the backend emits an activity/waiting-for-input signal is the regular path (adding that signal = a separate decision).
- **No color literals (variable-only)** — status is carried by the glyph shape and does not depend on color (e-ink contrast). The existing `AgentTree` statusColor is replaced by a shape-based one when switching to AgentList.
