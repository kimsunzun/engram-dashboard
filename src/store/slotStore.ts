import { create } from 'zustand'

interface Slot {
  id: number
  agentId: string | null
}

interface SlotState {
  slots: Slot[]
  focusedSlotId: number
  setFocusedSlot: (id: number) => void
  assignAgent: (slotId: number, agentId: string) => void
}

export const useSlotStore = create<SlotState>((set) => ({
  slots: [
    { id: 1, agentId: null },
    { id: 2, agentId: null },
  ],
  focusedSlotId: 1,
  setFocusedSlot: (id) => set({ focusedSlotId: id }),
  assignAgent: (slotId, agentId) => set(state => ({
    slots: state.slots.map(s => s.id === slotId ? { ...s, agentId } : s),
  })),
}))
