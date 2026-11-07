// ADR-0067: open/target state of the agent monitoring search popup (frontend-only UI state).
//
// ★Role★: slot right-click "agent monitoring" → holds only which slot the search popup is open targeting.
//   The placement itself (assign_agent) is not held here — the placement core is viewStore.assignAgent alone (§5 single control
//   surface, ADR-0067). This store holds only the pure display state "the popup is open and the target is (viewId, slotId)"
//   (no separate placement state — ADR-0067 invariant).
//
// ★Target = the right-clicked slot (explicit)★: open(viewId, slotId) carries the coordinates as is so placement does not
//   depend on focus (cuts focus-steal off at the source, ADR-0067). Right-click does not touch focused_slot_id, so this
//   store is unrelated to focus too.
//
// ★Per-window instance★: it is a zustand module store so each webview window (WebView2) has its own instance — the picker appears only in the
//   window that opened the popup (the command runs in the window that owns the right-clicked slot, so they match).

import { create } from 'zustand'

/** Coordinates of the slot the popup targets (the right-clicked slot). null when closed. */
export interface MonitoringPickerTarget {
  /** View id the right-clicked slot belongs to (comes from the tab override WindowLayout passed). */
  viewId: string
  /** Node id of the right-clicked slot (SlotContent assignment target). */
  slotId: string
}

interface MonitoringPickerState {
  /** Target coordinates if open, null if closed. The picker decides whether to mount by this value. */
  target: MonitoringPickerTarget | null
  /** A monotonically increasing counter that goes +1 on every open() call. WindowLayout passes it down as `key={openId}` so
   *  AgentMonitoringPicker is freshly remounted on every open (prevents a stale query/activeIndex flash, ADR-0067).
   *  The key point is that the value is preserved even while closed, so the key changes at the next open. */
  openId: number
  /** Open the popup — fix the target by carrying the right-clicked slot's coordinates (called from the §5 command). */
  open: (viewId: string, slotId: string) => void
  /** Close the popup (select · Esc · backdrop). Clear the target. */
  close: () => void
}

export const useMonitoringPickerStore = create<MonitoringPickerState>((set, get) => ({
  target: null,
  openId: 0,
  open: (viewId, slotId) => set({ target: { viewId, slotId }, openId: get().openId + 1 }),
  close: () => set({ target: null }),
}))
