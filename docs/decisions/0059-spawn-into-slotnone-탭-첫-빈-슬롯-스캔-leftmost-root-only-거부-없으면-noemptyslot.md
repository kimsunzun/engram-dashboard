# ADR-0059: spawn_into slot=None = scan for the tab's first empty slot (leftmost-root-only rejected — NoEmptySlot if none)

- 상태: 확정 (2026-07-09, basis: stage 5 commit 88134e2 + /review code full closed with PASS)
- 관련: TRD `docs/process/B-wezterm-tabs/TRD.md` §6 G9 (spawn_into slot policy) · `src-tauri/src/layout/manager.rs:498-532` (resolve_spawn_slot, SpawnSlotError) · `src-tauri/src/layout/tree.rs:52` (first_empty_slot_id) · ADR-0057 (tab ownership model) · step-log Phase 2 stage 5

> ★**The function locations named in 「관련」, 「결정」 and 「영향 / 불변식」 below are out of date**★ (note 2026-10-04, decision unchanged — the `tree::first_empty_slot_id` named in 「결정」 is no longer used for automatic placement · see ② below). Storage management P3b1 (`5a2cf7f` · TRD `docs/process/S21-storage/trd.md` §6-2) changed three things. ① `resolve_spawn_slot` became a `ViewManager` method (`ViewManager::resolve_spawn_slot(view_id, slot)` — `src-tauri/src/layout/manager.rs`) — it is still a policy decision that knows nothing about spawn, locks or emit. ② The free/occupied decision is the single `ViewManager::slot_is_free` — **free = `SlotContent::Empty` ∧ the unknown-content side table (`unknown_content`) has no entry for that slot.** Unknown slot content saved by a newer version stands as `Empty` in memory but counts as occupied — so that automatic placement does not overwrite that raw text. The rules of `slot=None` (the first empty slot in a pre-order walk), `NoEmptySlot` and `SlotOccupied` are unchanged, and only the decision of 「빈」 became narrower. `tree::first_empty_slot_id` looks only at the tree and so answers that a side-table slot is empty, so it is not used for automatic placement (the comment on that function). ③ `tree::assign_in_tree` was removed — the single content write to the tree is `tree::set_in_tree`, and `ViewManager` calls it only through its own write statement `write_slot_content`. **The overwrite semantics of `assign_agent` are unchanged** — the occupancy defense is still done only at the `resolve_spawn_slot` layer.

## 맥락
When `slot` is not specified in `spawn_into`, we must decide which slot of the target tab the agent is assigned to. TRD §6 G9 wrote "the **empty root slot** of the target tab", and this wording splits into two readings: (a) look at only one cell, the root (topmost/leftmost) slot, and fail if it is occupied — right for a newly created tab (1 empty slot), but in a split tab it fails when the left is filled even if the right is empty, (b) walk the tree and find the first empty slot. G9 was an item put in to stop the coder from guessing, but this point was still ambiguous.

## 결정
`slot=None` picks **the first empty slot by a pre-order (left-first) walk of the tree** (new `tree::first_empty_slot_id`). If there is no empty slot at all, it fails with `SpawnSlotError::NoEmptySlot` — **no automatic split or overwrite is done**. `slot=Some(s)` is separate: `SlotNotFound` if `s` is not in the tree, `s` if it is empty, `SlotOccupied` if it is occupied (no automatic split/replace). This reading settles the wording "empty root slot" in TRD §6 G9.

## 거부한 대안
- **leftmost-root-only (a) — check only one cell, the root/leftmost slot.** In a split tab, if the left is occupied it fails with `SlotOccupied` even when the right is empty — users and the LLM get confused by "why does it fail when there is an empty spot?". It conflicts with the intent of a placement-specified spawn (put it in an empty spot), so rejected.
- **Automatic split/overwrite when there is no empty slot.** It silently causes a layout change (a split) that the caller did not specify, or destruction of an existing agent (an overwrite) — hard to undo and outside the intent. Fail-loud with `NoEmptySlot` so the caller specifies the split explicitly.

## 근거
- In round 1 of the `/review code full` 2-family adversarial review, the ambiguous slot policy and missing tests were pointed out → closed with PASS after introducing `first_empty_slot_id` + adding unit tests (single-empty/single-occupied/skips-occupied-leftmost/all-occupied).
- The `resolve_spawn_slot`/`first_empty_slot_id` path tests passed in a throwaway verbatim-mount harness (stage 5 qa).
- User decision: "2-b" (first-empty-slot scan) was settled in this session's conversation.

## 영향 / 불변식
- `resolve_spawn_slot` (pure, `manager.rs:498-532`) is the single source of the slot policy — spawn_into performs the occupancy check itself with it before assignment.
- **Do not change the overwrite semantics of `assign_agent`/`tree::assign_in_tree`** — `move_slot_to_window` and others depend on them. The occupancy defense only at the resolve_spawn_slot layer.
- `NoEmptySlot`/`SlotOccupied` arise at the assignment stage **after the spawn has already succeeded**, so the command reports these errors while keeping the agent alive without killing it (it can be reattached via `list_agents` — layout.rs `alive_err`).
