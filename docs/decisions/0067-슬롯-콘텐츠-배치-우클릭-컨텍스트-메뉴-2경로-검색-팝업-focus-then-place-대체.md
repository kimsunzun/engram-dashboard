# ADR-0067: Slot content placement = right-click context menu, 2 paths + search popup (replaces focus-then-place)

- 상태: 확정 (2026-07-10, basis: user decision (right-click 2 paths + search popup · spawn = the tree's responsibility) + `/research medium` OSS survey (VS Code, JetBrains, tmux, wezterm, i3/sway, dual-pane mc/TC, WCAG 2.5.7) + 4 Codex adversarial review FIX items)
- 관련: Amends ADR-0066 (Decision 2 (focus-then-place placement) + Decision 5 (cross-window place target) → replaced by right-click context menu placement) · CLAUDE.md §5 (LLM-first control) · ADR-0011 (assign_agent) · ADR-0060 (SlotContent union) · ADR-0063 (set_slot_content) · ADR-0064/0065 (slot menu single contribution API · descriptor hideOn/children) · ADR-0035 (layout authority) · `src/commands/slotContentCommands.ts` · `src/components/slot/SlotContextMenu.tsx` · `src/components/agent/AgentList.tsx` (openInFocusedSlot) · step-log

## 맥락

ADR-0066 Decision 2 set placement as **focus-then-place** ("click a slot = focus → double-click a tree agent → assign to the focused slot"). After implementing and verifying Decision 1 (click-to-focus), designing the actual placement flow exposed a fundamental flaw:

**The agent tree is also inside a slot, so the moment you click the tree to pick an agent, that click steals focus to the tree slot** (click-to-focus = clicking anywhere in a slot focuses that slot). So "place in the focused slot" points at the tree slot itself and placement breaks (focus-steal). This breaks the premise of focus-then-place (that the target focus is preserved while operating the tree). Only right-click ("열기") (open) does not steal focus, so it was the only thing that worked.

Because this is a major redesign of the §5 placement control surface, we built the basis with `/research medium` (OSS survey → option set), and the user chose the direction.

## 결정

Placement is done by **a right-click context menu with 2 paths + a search popup**. Right-click does not move focus, so focus-steal is blocked at the root.

1. **Path 1 (slot → agent):** slot right-click → "에이전트 모니터링" (monitor agent) → **search popup** (command-palette style — a search box + a filtered list of running agents) → on selection, `assign_agent` to **the slot that was right-clicked**. Target = the right-clicked slot (explicit).
2. **Path 2 (agent → slot):** tree agent right-click → "열기" (the existing `openInFocusedSlot`) is kept — it is a right-click, so the focus slot set by the preceding left-click is preserved.
3. **Spawn = the tree's responsibility:** remove "생성" (create) (`slot.createAgentHere`) from the slot content-fill menu. Creating a new agent happens only in the tree (double-click a reserved profile + `agentlist.createAgent` in the agent_list slot menu).
4. **Drag-and-drop = later.** Added later as mouse sugar on top of the same assign.
5. **§5 single control surface:** the placement core = a single `assign_agent` (view/slot/agent) — a human right-click, the popup and the LLM all move the same command. Opening and selecting in the popup are also registered in the command registry.

## 거부한 대안

- **focus-then-place (ADR-0066 Decision 2, replaced by this ADR):** a tree click steals focus to the tree slot, so "place in the focused slot" points at the tree itself (focus-steal). Mature tools avoid this by architecturally separating the source (sidebar) from the target (editor group), but we have uniform slots, so we lack that separation. Right-click blocks focus-steal at the root and is simpler. focus-then-place discarded.
- **arm-then-drop (pick up then put down):** tree click = pick up, slot click = put down. It avoids focus-steal but is not a standard desktop idiom (a modal armed state + a visual affordance and Esc are needed), and a right-click context menu is more standard and simpler. Rejected.
- **Backend tracking of a cross-window last_focused_window (ADR-0066 Decision 5):** right-click makes the target explicit (the right-clicked slot), so resolving "last focus = target" is unnecessary. Codex: risk of wrong placement from accidental focus + the cost of adding backend state. Since the context menu is target-explicit, we do not create the `last_focused_window` state at all. Rejected as unnecessary.
- **Role tagging by content kind (treating the tree specially as "source ≠ target"):** imitating the VS Code source/target split would require treating slots specially by content kind, which breaks the "all slots uniform" principle (Codex's point). Right-click is independent of content kind, so uniformity is kept. Rejected.
- **Drag-only:** WCAG 2.2 SC 2.5.7 (a drag feature requires a single-pointer alternative) + already deferred to later in ADR-0066. The context menu (single-pointer click) is the accessibility path. Drag is later sugar. Drag-only rejected.

## 근거

- **Research medium (OSS survey + 4 Codex FIX items):** every mature tool uses a **source ≠ target split** so that a source click does not overwrite the target. The standard non-drag placement path = the context menu **"open in…/send to"** (VS Code "Open to the Side" `Ctrl+Enter`, Windows "Send to", mc `Alt-O`). Codex corrections: ① focus and the placement target are separate concepts ② WCAG 2.5.7 requires a **single-pointer** alternative, not a keyboard one (right-click satisfies it) ③ avoid special-casing by content kind ④ cross-window last-focus risks accidental placement. **A right-click context menu avoids all four points.**
- **Reuse of existing infrastructure:** on top of the slot menu single contribution API (ADR-0064/0065) + SlotContent/assign (ADR-0060/0011), only a menu item + a search popup are added — nothing new in the backend (`assign_agent` as is).
- **Reinterpreting click-to-focus (ADR-0066 Decision 1):** the click-to-focus that is already implemented and committed is not discarded — it stays as a visual selection indicator but **sheds its role as a placement mechanism** (the placement target is the right-clicked slot). This ADR replaces only its original justification (the foundation of focus-then-place).

## 영향 / 불변식

- **Placement target = the right-clicked slot (explicit)** — it does not depend on focus. `focused_slot_id` is separate from placement (Codex: focus ≠ placement target). We do not create the `last_focused_window` backend state.
- **§5 single control surface:** placement = a single `assign_agent` (ADR-0011) — a human, the popup and the LLM are the same. If the popup/menu bypasses this command and creates separate placement state, it is a §5 violation (rejected in review).
- **Right-click leaves focus unchanged** — if opening the context menu/popup changes `focused_slot_id`, focus-steal recurs (rejected in review).
- **Spawn only in the tree** — do not re-add a spawn item to the slot content-fill menu (creation = the tree's responsibility).
- **Drag is later** — when added, it goes through the same assign (no separate state, §5).
- A `// ADR-0067` anchor on load-bearing paths (slot menu contribution, popup, on-select assign).
