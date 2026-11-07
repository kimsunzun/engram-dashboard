// ADR-0239: expand state of chat tool groups — holds only the value a human · the LLM picked per slot (a group not picked is collapsed).
//   Frontend-only · in-memory so it resets on a webview refresh (same level as layout — CLAUDE.md 「LLM-우선 제어」).
//   ★Resubscribe · replay do not clear it★ — the group key is the same after replay (`components/slot/chat/toolRuns.ts`) so the picked value
//   is reattached as is. Only a change of agent (`bind`) and the conversation being cleared for a new incarnation (`clear`) empty it.
//   ★Bound = that slot's conversation view is currently mounted in this window★ — the conversation view (RichSlot) calls `bind` in its mount effect and calls the
//   release function it got back in its cleanup. For a slot that is not mounted, `setOpen` answers `false`, so a command does not answer success for a
//   slot with no view (ADR-0167 — the sibling `slot.scrollToBottom`'s handle registry is also removed on unmount).
//   ★The one path that changes expansion is `setOpen`★ — both the human toggle and the LLM command `chat.toolGroup.setExpanded` (`commands/chatCommands.ts`)
//   change it only through this. Do not make a second state path (CLAUDE.md 「LLM-우선 제어」).

import { create } from 'zustand'

/**
 * One slot's entry — `open` = group key → picked expansion (if there is no key that group is collapsed) · `mount` = marker of the current binding (`null` =
 * not mounted — the picked value stays and is reattached as is when mounted again).
 */
export interface ToolGroupSlot {
  agentId: string
  open: Record<string, boolean>
  mount: number | null
}

export interface ToolGroupState {
  bySlot: Record<string, ToolGroupSlot>
  /**
   * A slot's conversation view was mounted — if the entry belongs to another agent, empty the picked values; if the same agent, leave them.
   * @returns a release function — call it on unmount (cleanup). It keeps the picked values and releases only the binding. Calling it twice, or after a new `bind`
   *   happened, does no harm (it does not release the new binding).
   */
  bind(slotId: string, agentId: string): () => void
  /**
   * Empty that slot's picked values (the binding stays). Call it only when the conversation is cleared for a new incarnation — calling it on resubscribe · replay
   * makes the expansion a human picked disappear.
   */
  clear(slotId: string): void
  /**
   * @returns whether it was applied — `false` if that slot is not bound now (never bound or released) and nothing is written.
   *   This widens the `void` of TRD S21-chat-ux §4-4 — the command uses this value to tell 「이 창에 그 뷰가 없다」 (this window has no such view).
   */
  setOpen(slotId: string, key: string, open: boolean): boolean
}

// Slot id · group key also come from outside (the LLM) — reading with `in` · a bare index would read a prototype name like `'constructor'` as an entry.
function own<T>(record: Record<string, T>, key: string): T | undefined {
  return Object.prototype.hasOwnProperty.call(record, key) ? record[key] : undefined
}

/** The expansion a human · the LLM picked — `undefined` = no picked value (also when there is no `slotId`). */
export function chosenOpen(
  state: Pick<ToolGroupState, 'bySlot'>,
  slotId: string | undefined,
  key: string,
): boolean | undefined {
  if (slotId === undefined) return undefined
  const slot = own(state.bySlot, slotId)
  return slot === undefined ? undefined : own(slot.open, key)
}

/** Is the group fully expanded now — if there is a picked value, that (both expanded · collapsed); otherwise collapsed. */
export function effectiveOpen(state: Pick<ToolGroupState, 'bySlot'>, slotId: string | undefined, key: string): boolean {
  // ADR-0263 decision 1: a running group is not expanded in full either (Claude Code convention) — with no picked value, only the last call's one line is
  //   shown while running (`ToolGroupRow`). ★Do not auto-expand a running tail group in full★ — it collapses the moment the turn ends and the conversation jumps by that height.
  return chosenOpen(state, slotId, key) ?? false
}

// A new marker per binding — so a release function releases only when it is its own binding (StrictMode's mount · cleanup · mount, late cleanup).
let lastMount = 0

export const useToolGroupStore = create<ToolGroupState>((set, get) => ({
  bySlot: {},
  bind: (slotId, agentId) => {
    const mount = ++lastMount
    const slot = own(get().bySlot, slotId)
    const open = slot !== undefined && slot.agentId === agentId ? slot.open : {}
    set(s => ({ bySlot: { ...s.bySlot, [slotId]: { agentId, open, mount } } }))
    return () => {
      const current = own(get().bySlot, slotId)
      if (current === undefined || current.mount !== mount) return
      set(s => ({ bySlot: { ...s.bySlot, [slotId]: { ...current, mount: null } } }))
    }
  },
  clear: slotId => {
    const slot = own(get().bySlot, slotId)
    if (slot === undefined || Object.keys(slot.open).length === 0) return
    set(s => ({ bySlot: { ...s.bySlot, [slotId]: { ...slot, open: {} } } }))
  },
  setOpen: (slotId, key, open) => {
    const slot = own(get().bySlot, slotId)
    if (slot === undefined || slot.mount === null) return false
    if (own(slot.open, key) === open) return true
    set(s => ({ bySlot: { ...s.bySlot, [slotId]: { ...slot, open: { ...slot.open, [key]: open } } } }))
    return true
  },
}))
