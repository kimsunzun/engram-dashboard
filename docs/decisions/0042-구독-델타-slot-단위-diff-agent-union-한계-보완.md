# ADR-0042: Subscription delta = slot-level diff (compensating for the agent-union limit)

- 상태: 확정 (2026-07-01, basis: S14 module ① fourth-round FIX-3)
- 관련: ADR-0040 · ADR-0041 · ADR-0043 · src-tauri/src/output_router.rs · src-tauri/src/commands/layout.rs · step-log S14

## 맥락
When the layout changes (window placement, agent assignment changes), how do we synchronize the daemon subscription and the output plane cursor? At first we diffed only the "set of currently visible agents" (agent-union), but it misses the case where the **number of windows** viewing the same agent changes:
- **1→2**: a new window starts viewing an agent that was already being viewed — the agent set is unchanged so the delta is 0 → the new window shows an empty screen.
- **2→1**: only one of several windows is closed — the agent set is unchanged so the delta is 0 → a leak of the closed window's cursor.

## 결정
Compute the subscription delta (`SubscriptionDelta`) as a **slot-level ((window, agent) pair) diff**.
- In addition to the agent-level toggles (`to_subscribe`/`to_unsubscribe` — for emitting the daemon wire subscription),
- capture the **slot-level changes** separately: `slots_to_replay` (a newly appeared (window, agent) → replay immediately on mount) · `slots_to_drop` (a disappeared (window, agent) → discard the cursor, drop the content if it was the last).
- `OutputRouter` computes both axes (agent set ↔ slot set) together, and `send_subscription_delta` calls the four methods (subscribe/unsubscribe/replay_slots/drop_slots).

## 거부한 대안
- **Agent-union diff only** — it cannot catch 1→2 and 2→1 (partial window changes), so a new window shows an empty screen + a dead cursor leaks. (The fundamental limit in the context above.)
- **Rebuild all cursors at every layout (rebuild without a diff)** — it blows away cursors that are being delivered normally, causing duplicate replay and loss of progress. It throws away the advantage that a delta preserves normal cursors.

## 근거
In fourth-round FIX-3 we closed the 1→2/2→1 cases with a slot-level diff (the comments of `commands/layout.rs send_subscription_delta` and the `connection.rs DropSlots` arm). The full leak that slipped through a channel-full silent drop is absorbed separately by `sweep_orphans`/`reconcile_slots` (ADR-0043) — the roles are split: the delta is the normal path, reconcile is the safety net.

## 영향 / 불변식
- `SubscriptionDelta` carries `slots_to_replay`/`slots_to_drop` (`src-tauri/src/output_router.rs`).
- `send_subscription_delta` calls the four methods (`src-tauri/src/commands/layout.rs`).
- The slot key = (window_label, agent) = `ViewSlotKey`.
- **If violated (diffing only the agent-union):** changes in the number of windows viewing the same agent (partial mount/close) are missed → an empty screen · a cursor leak.
