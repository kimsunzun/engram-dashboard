# ADR-0064: Slot context menu = single contribution API (common target=star + content co-location + unified registration, command single source)

- 상태: 확정 (2026-07-10, basis: user decision + `/research light` (prior-art survey of VS Code/JetBrains/Zellij/Obsidian)) · 부분 폐기 by ADR-0140 (split command id) · 부분 폐기 by ADR-0252 (adding optional checked and enabled fields to the fixed SlotMenuItem schema, and, in the two-step invariant for new items, the assembly site subscribing to ctx)
- 관련: CLAUDE.md §5 (LLM-first control) · ADR-0055/0022 (command registry) · ADR-0060 (SlotContent union) · ADR-0035 (layout authority = src-tauri) · `src/commands/registry.ts` · `src/components/slot/SlotContextMenu.tsx` · `src/components/layout/ViewLayoutRenderer.tsx` · step-log "슬롯 메뉴 기여" · Amended by ADR-0065 (descriptor schema extension: hideOn exclusion condition + children 1-level submenu (partially realizes the deferred when-DSL through hideOn)) · Amended by ADR-0140 (split command id) · Amended by ADR-0252 (adding optional checked and enabled fields to the fixed SlotMenuItem schema, and, in the two-step invariant for new items, the assembly site subscribing to ctx)

## 맥락

The slot right-click menu was split into two approaches: ① the generic `SlotContextMenu` (9 hardcoded items, calls viewStore directly — not through the registry) ② content components (PresetPalette, AgentList) building their own menus and **suppressing** the generic menu with `stopPropagation`. Result: preset/tree slots lost the common slot operations (close, split, popout, empty) **entirely, a bug where they could not even be closed**. Also, command registration was scattered across 4 modules (`themeCommands`, `tabCommands`, `presetCommands`, `agentCommands`), and App.tsx side-effect imported each of them (loading was scattered).

Prior art (`/research`): VS Code is the right model — a command is defined **once** in `contributes.commands`, menus **only reference the command id** in `contributes.menus` (no redeclaration), core common items and extension items coexist in the same contribution point, and visibility comes from `contextValue` (content discriminator) + `when`. It maps 1:1 onto our command registry (ADR-0055).

## 결정

**The slot menu is assembled through a single contribution API. Common items and content items are registered through the same path (consistency).**

1. **Single contribution API** — `registerSlotMenu(target: SlotContentType | '*', items: SlotMenuItem[])`. `SlotMenuItem = { commandId, group, order }` (fixed schema). `target='*'` = all slots (common). `target=<SlotContentType>` = only slots of that content type.
2. **Common slot ops = command registration + `'*'` contribution (one central file):** promote `slot.split.h/slot.split.v/slot.popout/slot.empty/slot.close` to registry commands (moving the current SlotContextMenu hardcoded logic into commands), then `registerSlotMenu('*', [...])`. These commands receive the execution context (viewId·slotId·agentId) as **run(args)** and call viewStore actions (the palette, keybindings and the LLM run the same command with the context argument).
3. **Content contributions = command registration + menu contribution co-located in each content module:** e.g. `presetCommands.ts` registers the `preset.add` command + `registerSlotMenu('preset_palette', [{commandId:'preset.add', ...}])`. agent_list gets "에이전트 생성" (create agent) (picker→agent.spawn), the empty slot gets fill-ops ("에이전트 생성"→spawn+assign / "트리 열기" (open tree) / "팔레트 열기" (open palette)), and the agent slot gets "에이전트 종료" (terminate agent) (kill).
4. **Unified registration (2 layers):** ㉮ Inside the content = command + menu contribution in one module. ㉯ Loading = import every contribution module from a **single manifest** (something like `src/commands/contributions.ts`), and boot loads only that one (remove scattered imports from App.tsx). "New content = its module + one line in the manifest".
5. **Builder + single render:** `buildSlotMenu(contentType)` sorts `contributionsFor(contentType)` + the `'*'` contributions **deterministically by group·order** (regardless of registration order) → resolves the command id from the registry (title/run) → a single menu component renders it. ViewLayoutRenderer opens this on slot pane right-click.
6. **Content components do not own the pane menu:** remove PresetPalette's and AgentList's own pane menu + `stopPropagation` → pane right-click bubbles to the unified menu. **But AgentList's row-level menu is item-targeted, so it stays separate** (VS Code `view/item/context` vs `view/context` distinction) — only row right-click does stopPropagation.
7. **Fail-loud + type safety:** if a contributed `commandId` is not in the registry, it is a dev error (a typo or missing registration is caught immediately). `target` is typed `SlotContentType | '*'`, so contributing a nonexistent type = a compile error.

## 거부한 대안

- **Keep content-owned menus (the current state)** — each content has its own menu + stopPropagation → the common ops disappear (exactly this bug). Copy-pasting the common ones into each content gives duplication and drift. Rejected.
- **A central Record `Record<SlotContentType, id[]>` (option B)** — everything in one file (an overview and an exhaustive check as strengths), but adding new content needs a central edit → lower content cohesion. The user's decision = distributed contribution (option A, "each content in its own content"). Rejected.
- **JetBrains-style imperative `update()` (visibility code per item)** — not declarative, hard to test, and the same shape as the current "each builds its own menu → common lost" problem (risk of recurrence). Rejected.
- **Introduce a when-expression DSL now** — for the MVP, visibility by `target` (= the content.type discriminator) alone is enough (like VS Code `contextValue`). If a compound condition is needed, leave the door open to **only add** `when?` to the descriptor, but do not introduce it now (ADR-0060 union-extension philosophy = avoid over-engineering). Rejected (deferred).

## 근거

- **Consistent with §5 and ADR-0055:** a menu item = a reference to the single-source command → human clicks, keybindings and the LLM (`__engramCmd.run`) use the same id and handler. The "UI first, control later" gap is closed.
- **Prior-art validation (VS Code):** define the command once + menus only reference it, common/extension coexist, discriminated by contextValue = a proven textbook design. Ported as is onto our registry.
- **Consistency (the user's core requirement):** whether common or content, **the same `registerSlotMenu` API, the same descriptor, the same timing (import side effect), single manifest loading** → the registration side is fully unified.

## 영향 / 불변식

- **A slot menu item must reference a command id** — no direct store call from a menu (the current SlotContextMenu direct-call pattern is discarded). A new item = two steps, command registration + a `registerSlotMenu` contribution; the common (`'*'`) part, the builder and the menu component are not touched.
- **Common ops are the `'*'` contribution single source** — content does not redeclare the common ones (redeclaration = drift, rejected in review).
- **Unified registration** — content contribution modules are loaded only from the single manifest (no scattered imports). Adding new content = one line in the manifest.
- **Row-level menus are the exception** — the AgentList row menu is item-targeted, so it is separate from the pane's unified menu (this separation is intentional — do not confuse them).
- **Fail-loud** — contributing an unregistered commandId = a dev error. A target typo = a compile error.
