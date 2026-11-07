# ADR-0078: The render mode is decided and fixed at agent creation (per-activation activation override discarded)

- 상태: 확정 (2026-07-13, basis: user decision + live GUI measurement + /review code full, /qa PASS)
- 관련: CLAUDE.md §5(LLM 제어) · ADR-0044 (output_format render mode) · ADR-0064/0065 (single contribution API for the slot menu, 1-level submenu container) · ADR-0076/0077 (activation resume, fresh-fallback) · `src/commands/agentCommands.ts` · step-log 2026-07-13

## 맥락
When activating a reserved node (a claude reserved profile), the question was where to decide the render mode — Terminal (PTY/xterm) vs StreamJson (headless NDJSON→RichSlot chat). The two modes themselves already exist through ADR-0044, and `createClaudeProfile` takes `output_format` at creation and persists it in the profile. The initial implementation attached a [클로드 터미널/클로드 JSON] ([Claude Terminal / Claude JSON]) submenu to the tree right-click "활성화" (Activate) so the mode was chosen **at every activation**, a per-activation override (the intent was to keep the stored profile's output_format immutable).

## 결정
The render mode is **decided at the time the agent (profile) is created and immutable afterwards** — the mode is an essential property of the agent.

- Expand the tree pane background right-click "에이전트 생성" (Create agent) into a 1-level submenu **[클로드 터미널 생성 / 클로드 JSON 생성]** ([Create Claude Terminal / Create Claude JSON]) (the ADR-0064/0065 slot menu container). Each child calls `createClaudeProfile` with `'Terminal'`/`'StreamJson'` and persists output_format in the profile (reusing ADR-0044).
- Activation is a single "활성화" (Activate) (no mode selection) — it spawns with the stored mode as is.
- §5 (LLM control): 3 creation commands — `agentlist.createTerminal` (`'Terminal'`) · `agentlist.createJson` (`'StreamJson'`) · `agentlist.createAgent` (a parameterized primitive, `args.outputFormat ?? 'StreamJson'`, an input outside the valid values throws fail-loud before the dialog opens). The menu references only command ids (the ADR-0064 invariant).

## 거부한 대안
- **A per-activation render mode override** — a way to choose the mode at every activation through an activation right-click submenu while leaving the stored profile immutable (`effective_command` threading + a protocol `SpawnProfile.output_format` override field + a core `with_output_format_override`). Discarded for three reasons:
  - **① Conceptual mismatch** — the render mode is an essential property of the agent, so the same reserved node changing character at every activation is a mismatch (user decision 2026-07-13).
  - **② Implementation complexity** — to keep the stored profile immutable, the override had to be wired beyond the frontend into the protocol and core (the fixed-at-creation way ends with frontend changes only — because createClaudeProfile already takes output_format).
  - **③ Measured defect** — on the early resume exit → fresh-fallback (ADR-0077) path the override was dropped, leaving a **half-working** state in which activating with Terminal still spawned StreamJson (live measurement 2026-07-13). In the fixed-at-creation way the stored value is the only source, so this defect is eliminated at the source.
  - **Preservation** — the attempted code is preserved in `git stash@{0}` ("adr-0078 per-activation override").

## 근거
- User decision (2026-07-13): "Once created, it must not be possible to use another mode; fixed after creation is right."
- Live GUI measurement: confirmed that the per-activation override leaked from Terminal to StreamJson on the fresh-fallback path and so worked only halfway (process command line ground truth).
- Gates: /review code full (cross-family, doc-aware+blind) PASS (input validation fail-loud FIX applied) · /qa PASS (vitest 613 green · tsc 0 · core isolation 0) · live GUI measurement of the creation submenu PASS.

## 영향 / 불변식
- **Frontend-only change** — no protocol/core/daemon (crates/**) diff. ADR-0044 (output_format render) · ADR-0064/0065 (slot menu) · ADR-0076/0077 (activation resume, fresh-fallback) remain valid (not discarded) — the activation path is exactly as at HEAD.
- A mode change is done by **recreation**. A command that edits an existing profile's output_format after the fact is out of scope for now (a separate ADR if needed).
- Anchors: `src/commands/agentCommands.ts` (`createReservedProfile` · the `create*` commands · the `registerSlotMenu('agent_list', ...)` container). This file is the single point of the fixed-at-creation contract — attaching a mode selection submenu to activation again breaks this decision.
