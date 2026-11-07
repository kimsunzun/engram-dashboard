# ADR-0073: Excluding control slots (tree, palette) from focus — restricting click-to-focus to content slots

- 상태: 확정 (2026-07-13, basis: real-UX cdp measurement PASS + 2-reviewer adversarial review) · 부분 폐기 by ADR-0224 (the clause deferring backend focus enforcement)
- 관련: CLAUDE.md §5 (LLM control surface · hands/brain) · **ADR-0066 (click-to-focus) refinement** · ADR-0067 (right-click focus invariant) · ADR-0060 (SlotContent union) · ADR-0035 (layout backend authority) · `src/components/layout/ViewLayoutRenderer.tsx` (onClick gate) · `src/components/agent/selectOpenTarget.ts` · `src/components/agent/AgentList.tsx` (openInFocusedSlot) · step-log · Amended by ADR-0224 (the clause deferring backend focus enforcement)

## 맥락
The tree row right-click "열기" (open) assigns a running agent to the focus slot (`focusedSlotId`) of the active view (`assignAgent`). But the slot pane's click-to-focus (ADR-0066) **allows bubbling**, so when a tree node is left-clicked, that click bubbles up to the tree slot pane and **the tree slot becomes focused.** Then the right-click "열기" (right-click does not touch focus — ADR-0067) **picked that tree slot as the target and overwrote the tree with the agent terminal** (even when another empty slot existed). Reproduced and confirmed with the live DOM (user report).

## 결정
**Exclude the control slots (`agent_list`=tree, `preset_palette`=palette) from the focus concept** (frontend only):
1. **Make the click-to-focus gate an allowlist** — the `ViewLayoutRenderer` onClick calls `focusSlot` only when the slot content is a content slot (`empty`/`agent`), and skips control slots (and future variants). The discriminator shares `isContentSlot` from `selectOpenTarget` (a single classifier). The bubbling itself is kept — internal interactions (tree buttons, palette) still fire as before.
2. **Make the "열기" target selection a pure function** — `selectOpenTarget(layout, focusedSlotId)`: if the focus is a content slot, that slot → otherwise (a control slot or focus=null) the first `empty` slot → if none, `null` (a failure toast). Since this function **structurally excludes control slots from the target**, "열기" overwriting the tree/palette is impossible regardless of the backend focus state.

## 거부한 대안
- **Filter only at the open point (focus model unchanged)** — filter control slots only in `openInFocusedSlot` and pick an empty slot. Dropped: the focus ring would still show on the tree, and other consumers of `focusedSlotId` such as directional movement would still sit on the control slot, an inconsistency that remains (the root cause — that the control slot gets focused — is not fixed).
- **Force-reject control slots in the backend `focus_slot`/`fixup_focus` (the authority owner — ADR-0035)** — the most fundamental (consistent down to the focus ring). **Deferred (defer, user decision 2026-07-13 — frontend only this time).** Avoids a Rust change. What remains = if the backend emits the tree as focused (the first slot of the default main layout = AgentList), the focus ring shows on the tree, a **purely visual residue** (no effect on behavior — `selectOpenTarget` blocks the clobber). To be handled together when the backend tree/preset composition is reconsidered.
- **Gate denylist (listing `agent_list || preset_palette`)** — dropped: when a future control variant (FileTree/ControlPanel, ADR-0060) is added, it silently becomes a focus target. An allowlist (content slots only) is future-safe (review nit applied).

## 근거
- **Real-UX cdp measurement (PASS):** after a real click on the tree, the tree slot is `focused: false` (the gate works) · after a real click on right-click→"열기", the tree is `treeStillPresent: true` (not clobbered). Verified with real DOM clicks, context menu and menu click, not an invoke smoke test.
- **2-reviewer adversarial review (different families):** doc-aware (worker-senior)=FIX · cross-family (Codex effort high)=PASS — agreement that the production logic is correct. FIX applied (allowlist) + backend enforcement=defer. The `selectOpenTarget` pure-function unit test covers every branch (focus=content/control/null · stale · nested split), vitest 598 green.

## 영향 / 불변식
- **click-to-focus focuses only content slots (`empty`/`agent`)** (an ADR-0066 refinement — control slots excluded). The discriminator = `isContentSlot` single source (`selectOpenTarget.ts`). Future control variants are automatically non-focusable because of the allowlist.
- **"열기" (`openInFocusedSlot`) goes through `selectOpenTarget` and does not target control slots** — the tree/palette cannot be clobbered (regardless of the backend focus state).
- ADR-0067 (right-click focus invariant) and ADR-0035 (layout backend authority — no optimistic update) are kept. Focus itself is still the `focusSlot`→invoke→emit authority loop.
- **Unresolved (deferred):** the backend `focus_slot`/`fixup_focus` still allows focusing a control slot → when the backend assigns the focus, a focus ring shows on the tree, a visual residue (no effect on behavior). Full consistency needs backend enforcement (a separate decision — together with reconsidering the backend tree/preset composition).
