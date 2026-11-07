# ADR-0065: Slot menu descriptor extension — hideOn exclusion condition + children 1-level submenu (empty-slot trimming + folding of content-fill items)

- 상태: 확정 (2026-07-10, basis: user decision (option "trim + content submenu") + `/research light` OSS survey (VS Code, JetBrains, iTerm2, Windows Terminal, GNOME HIG)) · 부분 폐기 by ADR-0228 (the slot.popout empty-cell trimming in Decision 1)
- 관련: Amends ADR-0064 (descriptor schema extension: hideOn exclusion condition + children 1-level submenu (partially realizes the deferred when-DSL through hideOn)) · CLAUDE.md §5 (LLM-first control) · ADR-0060 (SlotContent union) · `src/commands/slotMenu.ts` (SlotMenuItem/buildSlotMenu) · `src/commands/{slotCommands,slotContentCommands}.ts` (contributions) · `src/components/slot/SlotContextMenu.tsx` (render) · Amended by ADR-0228 (the slot.popout empty-cell trimming in Decision 1)

## 맥락

The right-click menu of an empty slot (SlotContent=empty) was a cluttered flat list of 8 items: 3 content-fill items (에이전트 트리 열기 (open agent tree)·프리셋 팔레트 열기 (open preset palette)·에이전트 생성 (create agent); `target='empty'`, group=content) + 5 common slot-ops (가로/세로 분할 (split horizontally/vertically)·팝업 분리 (popout)·비우기 (empty)·닫기 (close); `target='*'`, group=slot-ops). Two problems:

1. **Meaningless items shown in an empty slot** — `slot.empty` (비우기) is a no-op that empties an already-empty slot again, and `slot.popout` (팝업 분리) also has little value for popping out an empty cell. Both are registered universally with `target='*'`, so they are shown unconditionally regardless of content type. ADR-0064 decides visibility by `target` alone and had no means (the deferred `when`) to "exclude from a specific type".
2. **Content vs layout mixed on one plane** — "what do we put in this cell" (content fill) and "what do we do to this cell" (layout operations) are mixed in 8 lines of one menu, so the hierarchy is weak (at the edge of the GNOME HIG 3~12 upper bound).

`/research light` OSS survey (grounding = official docs/issues of 4 apps + GNOME HIG): mature multi-pane apps use **flat divider groups without exception**, and **deliberately avoid** turning layout-ops into submenus (the split-submenu proposals of VS Code #247761 and Windows Terminal #18137 were both not merged). The only place a submenu is worth it is the **"New >" pattern**, where content types grow dynamically. GNOME HIG advises 3~12 for context menus and 3~6 for submenus, and discourages nesting.

## 결정

Leave the ADR-0064 single contribution API (`registerSlotMenu`, command single source, '*' common, unified registration) as is, and **extend only the descriptor schema, additively, in 2 ways**.

1. **`hideOn?: SlotContentType[]` (exclusion condition)** — hides that item in slots of specific content types. `hideOn: ['empty']` on `slot.empty` and `slot.popout`. The universal `target='*'` registration is kept (honoring the common-ops single-source invariant), and only specific types are removed. The full `when` string DSL is not introduced; the visibility condition deferred by ADR-0064 is partially realized by this narrow type array.
2. **`children?: SlotMenuItem[]` + container `title` (1-level submenu)** — fold the 3 content-fill items (`slot.fill.agentList`, `slot.fill.presetPalette`, `slot.createAgentHere`) into a "새 콘텐츠 ▶" (new content) container item. A container has `title`+`children` without `commandId` (an executable item has `commandId`, a container has `children`). A place to hang future backend types (codex/gemini) when they are added.
3. **Render** — expand 1 level as a hover flyout in `SlotContextMenu`. Reuse the recent viewport clamp logic (no ADR assigned, `aebfa86`) so it expands to the left on right overflow. **Nesting is limited to 1 level** (no children inside children).
4. **§5 LLM control unchanged** — `hideOn` and `children` are only render/visibility **presentation**; commands such as `slot.fill.*` can still be called directly as the registry single source (`__engramCmd.run`). Folding into a submenu opens no gap in the LLM control surface.

Final empty-slot menu: `새 콘텐츠 ▶` · ─── · `가로 분할` · `세로 분할` · `닫기` (5 lines, the 3 fill items go into the submenu).

## 거부한 대안

- **Fold the layout-ops (split/popout/close) into a submenu** — VS Code (#247761) and Windows Terminal (#18137) both deliberately did not merge the split-submenu proposal. They are frequently used verbs, so one more click level is a loss, and GNOME HIG also discourages nesting. What the user wanted was also "fold the fill + trim the meaningless", not layout nesting. Rejected.
- **Introduce the full `when` string DSL now** (what ADR-0064 deferred) — the current need is only one: "exclude a specific content type". No gain against the parser/evaluator cost (§0 criterion: no compound condition has appeared = uncertain → not even a shell, keep it uninstalled). The `hideOn` type array is enough, and when a compound visibility condition actually appears, promote it to a DSL then. Rejected (deferral kept).
- **Remove `slot.empty`/`slot.popout` from '*' and redeclare them per content type** — violates the ADR-0064 invariant "common ops = '*' single source, no content redeclaration" (drift, rejected in review). The hideOn exclusion achieves the goal while keeping the '*' universality. Rejected.
- **Support nesting 2 or more levels deep** — GNOME HIG discourages nesting + the safety-triangle mouse tracking problem. Limited to 1 level. Rejected.

## 근거

- **OSS survey grounding:** flat dividers + a content/layout section split = common to VS Code, JetBrains, iTerm2 and Windows Terminal. A submenu is worth it only for dynamic content types ("New >") — which fits exactly with our future multiple backends.
- **§0 criterion (risk × horizon):** `hideOn` (a narrow type field, low risk, easy to extend) now, the full `when`-DSL (uncertain, no compound condition has appeared, expensive to undo) deferred. A middle between excessive abstraction (DSL) and too little (hardcoded branches).
- **Backward compatible:** `hideOn` and `children` are both optional → existing contribution code unchanged. The core of ADR-0064 (single contribution, command single source, unified registration, row-level exception) all remains valid; only an additive schema extension.

## 영향 / 불변식

- **descriptor schema** = `{ commandId?, title?, group, order, hideOn?, children? }`. With `commandId` it is an executable item (title is resolved from the registry); with `children` it is a container (`title` required, no `commandId`). Neither or both is a dev error (fail-loud).
- **Submenu limited to 1 level** — no `children` inside `children` (render, GNOME HIG). A violation is a dev error.
- **hideOn = for exclusion (subtraction) only** — not an allowlist. It keeps the `target='*'` universality while removing only specific types. Common ops are still the '*' single source (the ADR-0064 invariant is kept).
- **§5 unchanged** — `hideOn`/`children` are only a render presentation extension. The direct command call paths (palette, keybindings, LLM `__engramCmd.run`) are unchanged. A command must always be callable regardless of whether the menu lists it.
- **Backward compatible** — an existing `{commandId, group, order}` contribution stays valid as is (hideOn/children unspecified = existing behavior).
