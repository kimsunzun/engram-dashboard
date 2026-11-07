# ADR-0071: Persistence registry mutate lock discipline — run store.save while holding the map lock (concurrent-rename stale-snapshot race fix)

- 상태: 확정 (2026-07-12, basis: surfaced by cross-family (Codex) → fix → re-verification PASS · concurrency regression test) · 부분 폐기 by ADR-0291 (the leaf write_lock in the lock order, and the apply/save commit order of mutate)
- 관련: ADR-0006 (lock order — **a separate domain**) · ADR-0070 (§5 rename exposure opened the concurrency window) · ADR-0061 · `crates/engram-dashboard-core/src/agent/preset.rs` (`PresetRegistry::mutate`) · `.../agent/profile.rs` (`ProfileRegistry::mutate`, `mutate_if`, `observe_session_id`) · step-log · Amended by ADR-0291 (the leaf write_lock in the lock order, and the apply/save commit order of mutate)

## 맥락

The common change path `mutate` of `PresetRegistry`/`ProfileRegistry` was originally **"change under the map lock → only take a snapshot → release the lock → `store.save` outside the lock"** (the intent was to move disk IO out of the lock, per the existing comment design). It is harmless if only a single caller (a human UI — one at a time) is assumed.

But as ADR-0070 exposed rename as a §5 command (`RenamePreset`/`RenameProfile`), the LLM/orchestrator can now call create/rename/delete **programmatically, concurrently and back to back** — an interleaving window that a human hand cannot open becomes a real path.

## 결정

Bind `mutate` and `store.save` into **one critical section** — save **before** releasing the lock. The snapshot to save is the latest map just committed, so `persisted == observed` is guaranteed. For the path with no change (`observe_session_id`), add the conditional variant `mutate_if` (saves while holding the lock only when the closure returns `true`) to keep the saving of no-op disk writes.

**Fix 3 places with the same racy pattern**: `PresetRegistry::mutate` · `ProfileRegistry::mutate` · `observe_session_id` (the old code was `list()` outside the lock + save → delegated to `mutate_if`). create/delete/rename/set-autorestore all go through this common path too, so they become safe together.

## 거부한 대안

- **IO outside the lock (the existing design — save outside the lock)** — it opens the race. When two mutations overlap, in the order `A 스냅샷 → unlock → B 스냅샷 → unlock → B save → A save`, the in-memory state and the broadcast are the latest (B) but the disk is left stale (A), and the old value is loaded on restart (`persisted ≠ observed`). Worst case, A's snapshot cannot see B's insert and an **entry is lost**. The §5 concurrent calls make this window actually open. The benefit of a shorter lock-hold time is meaningless for local small-file IO.

## 근거

The cross-family reviewer (Codex, effort high) surfaced it, and the user confirmed it through §5 reachability (concurrent LLM calls).

- **No deadlock (unrelated to ADR-0006):** `store.save` takes only the store's internal leaf mutex (`write_lock`) and does not re-enter the registry → the lock order is one-directional `presets|profiles → write_lock`, with no cycle. The registry lock is **separate** from the session (sessions/core/status) lock domain, so it is not entangled with the ADR-0006 order (the reaper and manager call the registry after releasing the sessions guard).
- **advisory:** a map read blocks during save, but it is a local small file so it is negligible.
- **Regression sealed:** `concurrent_mutations_persisted_equals_final_map` (4 threads × 50 create+rename = 200 entries → the disk == the in-memory id set, contents match) + `save_writes_current_map_not_stale_snapshot`.

## 영향 / 불변식

- **Do not move the `store.save` of `mutate`/`mutate_if` back outside the lock** — that reverts to exactly the rejected alternative (the old code) and the stale-overwrite race recurs. Code anchor `// ADR-0071`.
- The registry change paths (create/delete/rename/set-autorestore/observe_session_id) all go through this common path — a new change API must go through `mutate`/`mutate_if` (no direct lock + save).
- This lock domain is **separate** from the ADR-0006 (sessions → internal) order (the store leaf lock). Do not hold the two domains overlapping in one path.
