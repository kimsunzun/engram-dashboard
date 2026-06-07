import { create } from 'zustand'

export type SplitDir = 'horizontal' | 'vertical'

export interface SlotNode {
  type: 'slot'
  id: number
  agentId: string | null
}

export interface SplitNode {
  type: 'split'
  dir: SplitDir
  children: LayoutNode[]
}

export type LayoutNode = SlotNode | SplitNode

let nextId = 3

// ── tree helpers ──────────────────────────────────────────────────────────────

export function findSlot(node: LayoutNode, slotId: number): SlotNode | null {
  if (node.type === 'slot') return node.id === slotId ? node : null
  for (const child of node.children) {
    const found = findSlot(child, slotId)
    if (found) return found
  }
  return null
}

export function getAllSlotIds(node: LayoutNode): number[] {
  if (node.type === 'slot') return [node.id]
  return node.children.flatMap(getAllSlotIds)
}

function splitInTree(node: LayoutNode, slotId: number, dir: SplitDir): LayoutNode {
  if (node.type === 'slot') {
    if (node.id !== slotId) return node
    return {
      type: 'split',
      dir,
      children: [node, { type: 'slot', id: nextId++, agentId: null }],
    }
  }
  return { ...node, children: node.children.map(c => splitInTree(c, slotId, dir)) }
}

function closeInTree(node: LayoutNode, slotId: number): LayoutNode | null {
  if (node.type === 'slot') return node.id === slotId ? null : node
  const kept = node.children
    .map(c => closeInTree(c, slotId))
    .filter((c): c is LayoutNode => c !== null)
  if (kept.length === 0) return null
  if (kept.length === 1) return kept[0]
  return { ...node, children: kept }
}

function assignInTree(node: LayoutNode, slotId: number, agentId: string): LayoutNode {
  if (node.type === 'slot') return node.id === slotId ? { ...node, agentId } : node
  return { ...node, children: node.children.map(c => assignInTree(c, slotId, agentId)) }
}

// ── initial layout ────────────────────────────────────────────────────────────

const initialLayout: LayoutNode = {
  type: 'split',
  dir: 'horizontal',
  children: [
    { type: 'slot', id: 1, agentId: null },
    { type: 'slot', id: 2, agentId: null },
  ],
}

// ── store ─────────────────────────────────────────────────────────────────────

interface SlotState {
  layout: LayoutNode
  focusedSlotId: number
  setFocusedSlot: (id: number) => void
  assignAgent: (slotId: number, agentId: string) => void
  splitSlot: (slotId: number, dir: SplitDir) => void
  closeSlot: (slotId: number) => void
}

export const useSlotStore = create<SlotState>((set) => ({
  layout: initialLayout,
  focusedSlotId: 1,
  setFocusedSlot: (id) => set({ focusedSlotId: id }),
  assignAgent: (slotId, agentId) =>
    set(s => ({ layout: assignInTree(s.layout, slotId, agentId) })),
  splitSlot: (slotId, dir) =>
    set(s => ({ layout: splitInTree(s.layout, slotId, dir) })),
  closeSlot: (slotId) =>
    set(s => {
      const newLayout = closeInTree(s.layout, slotId)
      const layout: LayoutNode = newLayout ?? { type: 'slot', id: nextId++, agentId: null }
      const ids = getAllSlotIds(layout)
      const focusedSlotId = ids.includes(s.focusedSlotId) ? s.focusedSlotId : (ids[0] ?? 1)
      return { layout, focusedSlotId }
    }),
}))
