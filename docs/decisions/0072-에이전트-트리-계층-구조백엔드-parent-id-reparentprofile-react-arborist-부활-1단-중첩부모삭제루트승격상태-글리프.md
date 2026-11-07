# ADR-0072: Agent tree = hierarchical structure (backend parent_id + ReparentProfile) + react-arborist revived — 1-level nesting · parent deletion = root promotion · status glyphs

- 상태: 확정 (2026-07-12, basis: user interview decision — implementation follows)
- 관련: CLAUDE.md §5 (LLM control) · ADR-0014 (orchestration — this hierarchy is its visual and data foundation) · ADR-0018 (reserved = frontend merge) · ADR-0062 (status glyphs) · ADR-0070 (display_name override — the same additive pattern) · ADR-0071 (persistence lock) · `crates/engram-dashboard-core/src/agent/profile.rs` (`AgentProfile`) · `src/components/agent/mergeTreeNodes.ts` · `src/components/agent/AgentList.tsx`

## 맥락

The agent tree (`AgentList`), despite its name, is a **flat list** — the MVP had no hierarchy, so `react-arborist` was removed in favor of a flat list (the `AgentList.tsx` header comment). The user's next goal is orchestration: **A as the main, B·C·D under it**, with A shooting messages to B and B reporting back to A. The tree needs a parent-child hierarchy as the visual and data foundation for that. Even if it is only shown "on the surface for now", the arrangement must not scatter after a restart (the user), and the LLM must be able to control the tree composition (§5).

## 결정

1. **Parent-child = stored in the backend.** Add `AgentProfile.parent_id: Option<AgentId>` (`#[serde(default)]`) + a `ReparentProfile { child_id, parent_id: Option<AgentId> }` command (the same additive pattern as the rename slice ADR-0070). Under §5 the LLM/user assigns the parent through the same command. Mirrored in protocol/bindings.
2. **Render = revive react-arborist** (an already-installed dependency `^3.9.0`). Indentation, collapse/expand and drag re-parenting. `mergeTreeNodes` returns a `children` tree that groups children under their parent by parent_id instead of a flat concat.
3. **Nesting depth = 1 level** (A > B·C·D). A child cannot become a parent again (simplifies cycle handling).
4. **Parent deletion = children promoted to root** (orphan-to-root). When A is deleted, the parent_id of B·C·D is released to None and they go to the top level. Not a cascade delete (data is preserved).
5. **Display = hierarchy as the main axis + status glyphs.** The running/saved distinction is shown not by section but by the node glyph (● running / ○ reserved / ◐ spawning) — the hierarchy is the organizing axis, which avoids a conflict with sections. **Spawning = frontend synthesis** (there is no separate Spawning state in the backend — the frontend shows it between the spawn request and the first Running).

## 거부한 대안

- **A frontend-only cosmetic parent map** — a §5 violation (the LLM cannot control the tree composition) · not persistent (the arrangement scatters on restart) · discarded when orchestration routing is attached. The user chose backend storage with "it must not scatter when turned off and on".
- **Keep the flat list + indent only** — no collapse/expand or drag re-parenting. It would mean manually reimplementing tree features when `react-arborist` is already a dependency.
- **Multi-level (arbitrary depth)** — cycle prevention and re-parenting constraints get complicated. Excessive for the current A>BCD model → start with 1 level, extend if needed.
- **Keep sections (running/saved) + hierarchy inside a section** — if a parent and a child span different sections (A=running, B=reserved), they appear separated and the orchestration intuition breaks. Replaced by hierarchy as the main axis + glyphs.
- **Cascade on parent deletion** — risk of accidentally wiping a whole group. Root promotion is safe.
- **Add a new Spawning state in the backend** — changing the lifecycle state machine (ADR-0016/0019) has high cost and risk. It is a transitional display, so frontend synthesis is enough (move it to the backend later if measurement is needed).

## 근거

- **§5 (hands/brain separation):** the tree composition (who is under whom) must also be a handle the LLM controls → backend `parent_id` + command. A human drag is an auxiliary input to the same handle.
- **ADR-0014 foundation:** this hierarchy is the visual and data foundation of orchestration (A→B routing and reporting). If built cosmetically it would be discarded when routing is attached, so it is in the backend from the start.
- **Criterion §0 (low risk + long horizon → now):** the parent_id seam and command are a low-risk additive (the same shape as the ADR-0070 rename, already validated) and are needed for the long horizon (orchestration) → laid down now.
- **Reusing react-arborist:** it is reviving a dependency that was removed, not a new introduction.

## 영향 / 불변식

- Keep `AgentProfile.parent_id` `#[serde(default)]` — an old `agents.json` (without the field) → absorbed as None (no migration, the same discipline as ADR-0070).
- **`ReparentProfile` cycle prevention:** since it is 1 level, a check forbidding "child as parent" is mandatory (reject if the child is already someone's parent, reject self-parent, reject a nonexistent parent). Persistence goes through the ADR-0071 lock discipline (`ProfileRegistry` mutate).
- **On parent deletion, children's parent_id=None promotion** — deleteProfile sweeps the children and promotes them (one critical section). No orphan references (a parent_id that does not exist).
- **`mergeTreeNodes` keeps the ADR-0018 merge (running ∪ reserved) inside the hierarchy** — a parent and a child can each be running/reserved, distinguished by glyphs.
- react-arborist idAccessor=id · childrenAccessor=children. The subscription/state contracts (ADR-0046 etc.) are unchanged (only a display hierarchy is added).
- The Spawning glyph is frontend-derived — the backend state machine is unchanged (ADR-0016/0019).
