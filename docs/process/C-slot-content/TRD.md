# TRD: Slot content seam — `Slot.agent_id` → `Slot.content: SlotContent`

> Canonical decision = **ADR-0060** (typed union adopted, P2/P3 rejected). This document holds only the *how* (the implementation machinery).
> This slice = **the seam only** (behavior-identical). The UI for actual non-agent content (Tree/ControlPanel) is a **follow-up** (UX = user decision).

## 1. Scope
- **In:** replace `agent_id: Option<String>` of `LayoutNode::Slot` with `content: SlotContent` (`Empty | Agent { agent_id: String }`). End to end through the backend, wire and frontend render branch. Behavior is **exactly the same** (empty = Empty, assigned = Agent).
- **Out (follow-up):** the actual `FileTree`/`ControlPanel` variants + renderer + placement command, capability table, version/Unknown/migration on persistence. (Requirements recorded in ADR-0060 "영향/불변식".)

## 2. Change points (based on the Explore map — re-check file:line at the start of work)

### Backend (required)
- **`src-tauri/src/layout/types.rs`** — (a) new `SlotContent` enum (`#[serde(tag="type", rename_all="snake_case")]`, `#[ts(export)]`, `Empty` + `Agent { agent_id: String }`) + `//! `/`///` invariant comments + `// ADR-0060` anchor. (b) `LayoutNode::Slot { id, agent_id }` → `{ id, content: SlotContent }`. (c) `new_empty_slot()` → `content: SlotContent::Empty`. Convenience methods `SlotContent::is_empty()` / `agent_id() -> Option<&str>` recommended (simplifies call sites).
- **`src-tauri/src/layout/tree.rs`** — `find_slot` (returns `Option<&SlotContent>`), `first_empty_slot_id` (`matches!(content, SlotContent::Empty)`), `assign_in_tree` (parameter `SlotContent`, or split into assign/clear), destructuring patterns in 2 places (`{ id, content }`).
- **`src-tauri/src/layout/manager.rs`** — `slot_agent` (derive Option<String> from SlotContent), `assign_agent` (build `SlotContent::Agent{agent_id}`), `resolve_spawn_slot` (3-way: `Some(Empty)=Ok` / `Some(Agent{..})=SlotOccupied` / `None=SlotNotFound` — ADR-0059 invariant kept), `prepare_detached_view`, test patterns in 2 places (`content: SlotContent::Empty`).
- **`src-tauri/src/output_router.rs`** — `collect_agents`: destructure `LayoutNode::Slot { content, .. }` + `if let SlotContent::Agent { agent_id } = content { agent_id.parse::<AgentKey>() … }`, `Empty` explicitly ignored. (ADR-0041/0042/0046 routing invariant — only assigned slots receive.)
- **`src-tauri/src/commands/layout.rs`** — `spawn_into`/`assign_agent` commands: **keep the IPC wire parameter `agent_id: String`** (frontend contract unchanged); the manager assembles `SlotContent::Agent` internally. Minimal change.

### Wire (ts-rs — automatic)
- `src-tauri/bindings/LayoutNode.ts` + the new `SlotContent.ts` are **updated automatically by ts-rs regeneration** (no manual edits). `#[ts(export)]` is required. Regeneration = the binding export test/build.

### Frontend (1 required file)
- **`src/components/layout/ViewLayoutRenderer.tsx`** — switch the 4 places with `node.agent_id != null` to `node.content.type === 'agent'` (recommended: a `switch(node.content.type)` form). Agent lookup = `node.content.type === 'agent' ? node.content.agent_id : null`. The `agentId` prop passed to DomSlot/RichSlot/TerminalSlot·SlotContextMenu is non-null inside the branch.
- **Confirmed unchanged:** `viewStore.ts` (does not reference agent_id directly), `layoutTypes.ts` (re-export), `TerminalSlot.tsx` (receives the prop), `renderMode.ts`, `AgentTree.tsx` (outside the slot content model — a fixed side panel).

## 3. Tests (TDD — cumulative)
- **New:** `SlotContent` serde round-trip golden (`{"type":"empty"}` / `{"type":"agent","agent_id":"…"}`) · `is_empty`/`agent_id()` · ts-rs export shape assertion (if possible).
- **Updated:** move the existing layout unit/throwaway verbatim-mount harness (resolve_spawn_slot 3-way · first_empty_slot · assign · collect_agents routing) to the new type — the goal is a **no-behavior-change regression**.
- **Isolation:** keep the existing throwaway verbatim-mount approach for src-tauri pure logic (avoids 0xc0000139 — see the handoff do-not).

## 4. Acceptance criteria
1. `LayoutNode::Slot` has `content: SlotContent`, and the `Empty`/`Agent` round-trip is exact in internally tagged JSON.
2. Routing (collect_agents) · spawn_into · resolve_spawn_slot · assign have **the same behavior** (regression tests pass).
3. After ts-rs binding regeneration, the frontend `switch(content.type)` passes the type check.
4. GUI measurement: spawn_into → the agent is Running in the slot (= via the Agent variant), an empty slot = Empty. No regression in existing tab/split behavior.
5. Core isolation (`rg "use tauri" core` 0), fmt, all regressions (cargo member-scoped + vitest + tsc) green.

## 5. Risks
- A wrong conversion in the **load-bearing routing** (`collect_agents`) misroutes output → defended by regression tests, 2-family verification in /review code.
- A missed ts-rs regeneration causes wire drift → build/binding gate.
- Full `cargo test` hits 0xc0000139 (environment) — member-scoped + throwaway-mount workaround (handoff do-not).
