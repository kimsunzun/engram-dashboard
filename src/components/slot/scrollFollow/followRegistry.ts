// ADR-0242: slot id → scroll-follow handle — a module map inside this window (webview). The in-window command
//   (`commands/scrollCommands.ts`) finds the handle here. Do not create a new global handle (CLAUDE.md 「LLM-우선 제어」).

/** One slot's scroll-follow handle — the hook (`useScrollFollow`) makes it, the slot and commands call it. */
export interface FollowHandle {
  readonly pinned: boolean
  /** Pin to the bottom and write to the bottom. For a hidden tab only set pinned; the write happens when visible. */
  pin(): void
  /** Release following — leave the position as is. */
  unpin(): void
}

const handles = new Map<string, FollowHandle>()

/**
 * Registers, and returns the function that unregisters. ★Unregister clears the entry only if it is still its own handle★ — a new
 * mount on the same slot may have registered first (StrictMode double mount · render-mode swap · identity-key remount).
 */
export function registerFollow(slotId: string, handle: FollowHandle): () => void {
  handles.set(slotId, handle)
  return () => {
    if (handles.get(slotId) === handle) handles.delete(slotId)
  }
}

/** `undefined` if absent — this window has no conversation/text view of that slot mounted. */
export function getFollow(slotId: string): FollowHandle | undefined {
  return handles.get(slotId)
}
