# ADR-0063: Slot content placement control surface = generic set_slot_content command + boot default = AgentList·Empty split (fixed side panel removed)

- 상태: 확정 (2026-07-10, basis: user decision ("fully slotted") + layout-authority grounding) · 부분 폐기 by ADR-0140 (boot default layout direction vocabulary) · 부분 폐기 by ADR-0222 (Decision 2, boot default layout) · 부분 폐기 by ADR-0225 (the clause in Decision 3 keeping the tree-only window) · 부분 폐기 by ADR-0258 (only the per-cell usage merge path, within the single-path invariant for slot content changes and the rejection of per-variant dedicated commands)
- 관련: CLAUDE.md §5 (LLM-first control) · Architecture §5 · ADR-0060 (SlotContent union) · ADR-0035 (layout authority = src-tauri) · ADR-0055/0022 (command registry) · `src-tauri/src/layout/{manager.rs,tree.rs,types.rs}` · `src-tauri/src/commands/layout.rs` · `src/store/viewStore.ts` · `src/components/layout/AppLayout.tsx` · Amended by ADR-0140 (boot default layout direction vocabulary) · Amended by ADR-0222 (Decision 2, boot default layout) · Amended by ADR-0225 (the clause in Decision 3 keeping the tree-only window) · Amended by ADR-0258 (only the per-cell usage merge path, within the single-path invariant for slot content changes and the rejection of per-variant dedicated commands)

## 맥락

The agent tree (`AgentList`) and `PresetPalette` were defined as SlotContent variants in ADR-0060 (Slices A~C), but on screen they were still mounted as a **fixed left side panel** — the variants existed, but **there was no placement means to turn an empty slot into AgentList/PresetPalette**. The existing layout command (`assign_agent`) only plugs an *agent* into a slot, and there was no path to place non-agent content in a slot. So the "side panel = fixed chrome" remained, and the tree could not be moved, split or closed, and the LLM could not control the placement (§5 violation: the UI is fixed in the frontend, with no control surface).

User decision: **fully convert the agent tree into a real slot** (remove the fixed side panel). Separately, also remove the bottom StatusBar/dummy DiffPanel (S0 view-stage leftovers).

## 결정

1. **Add a generic `set_slot_content(view_id, slot_id, content: SlotContent)` command** — a single command that changes a slot's content to any of Empty/Agent/AgentList/PresetPalette. It mirrors the placement pattern of `assign_agent` (agent-only) but takes the content directly. `tree::set_in_tree` (new, mirrors `assign_in_tree`) → `ViewManager::set_slot_content` → `commands/layout.rs` wrapper (lock → mutate → bump_version → router.rebuild → send_subscription_delta → emit `layout:updated`+`window:tabs-updated`) → register in `generate_handler!` → frontend `viewStore.setSlotContent` → `invoke('set_slot_content')`. **This is the control surface shared by the LLM and people for §5 slot content placement.**
2. **Boot default layout = horizontal split [AgentList (left, small) · Empty (right)]** — instead of the single Empty slot of `ViewManager::new()`, the first view of the main window is made as a Split of a left `AgentList` slot + a right `Empty` slot. It reproduces the "tree always shown on the left" that the side panel did, as slots, but now the tree slot too can be moved, split and closed.
3. **Remove the fixed side panel (`Sidebar` in `AppLayout`)** — delete the left Allotment pane. AgentList exists only as a slot of the boot layout (2). The `/tree` dedicated window (TreePage) is kept (a separate window).
4. **Remove StatusBar + dummy DiffPanel** — both are S0 view-stage leftovers (a fixed 'Ready' string + a `function hello()` mockup, 0 real function, used only in AppLayout). Remove the AppLayout wiring and the component files. A real diff feature will be reimplemented when needed.

## 거부한 대안

- **Per-variant dedicated commands** (`set_slot_agent_list`, `set_slot_preset_palette` …) — commands explode each time a variant is added (clashes with the union-extension philosophy of ADR-0060). One generic `set_slot_content(content)` covers the whole union and absorbs new variants automatically.
- **Keep the fixed side panel + run the variant alongside** (the Slice C status quo) — the tree stays as frontend fixed chrome instead of a slot, violating §5 (all UI is LLM-controlled) + no move, split or popup. The user decided on a full conversion.
- **Boot default = a single AgentList slot** (no split) — then there is no slot to put a terminal/agent in, and you would have to split first every time (clunky first use). A split of left tree + right empty slot replaces the side panel UX without loss.
- **The frontend injects the tree by calling set_slot_content at boot** (instead of a backend default) — the layout authority is src-tauri (ADR-0035), so the backend must own the default layout too for consistency. Frontend injection means split authority + a boot race.

## 근거

- **§5 control surface completed:** slot content placement is now shared by the LLM and people through a single command (background right-click menu, `__engramCmd`, invoke are all the same handle). The "UI first, control later" gap is closed.
- **Layout authority consistent (ADR-0035):** the default layout and all placement changes are owned by the src-tauri ViewManager → the frontend only mirrors and renders.
- **Grounding check:** the `assign_agent`→`assign_in_tree` pattern already mutates slot content + emits → `set_slot_content` only generalizes that proven path.

## 영향 / 불변식

- **Slot content changes go through the single `set_slot_content` path** (agent placement keeps the existing `assign_agent` — spawn_into uses it). The frontend does not locally tamper with slot content directly (backend emit → mirror, ADR-0035).
- **Changing the boot default layout = a migration** — need to check how an existing saved layout (if any) interacts with the new default (layout persistence is currently unimplemented, so the default is generated every boot → safe).
- **AgentList/PresetPalette are now slot-only** — the fixed side panel mount is removed. If you close the tree, place it again by background right-click or set_slot_content.
- **StatusBar/DiffPanel removed** — if a real diff/status bar is needed, reimplement it (not a revival of the dummy leftovers).
