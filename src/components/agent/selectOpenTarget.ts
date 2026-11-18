// ★Control-slot focus excluded★: the tree (agent_list) and usage (usage) slots are not "work slots" and are not focus targets (LayoutLeaf's click-to-focus gate no longer focuses them). Even in existing/edge backend states (a control slot already focused, or focus=null) this function guards so "open" never overwrites a control slot with an agent terminal.

import type { LayoutNode, SlotContent } from '../../api/layoutTypes'

/**
 * ★Single classifier (allowlist)★: the single source of the "content slot" definition — selectOpenTarget and LayoutLeaf's click-to-focus gate use it **together**, so a denylist duality cannot arise (a future control variant silently becoming a focus/open target). A new control variant (ADR-0060 FileTree/ControlPanel etc.) is non-focusable automatically unless this allowlist catches it.
 */
export function isContentSlot(content: SlotContent): boolean {
  return content.type === 'empty' || content.type === 'agent'
}

function findSlotById(node: LayoutNode, slotId: string): Extract<LayoutNode, { type: 'slot' }> | null {
  if (node.type === 'slot') return node.id === slotId ? node : null
  return findSlotById(node.a, slotId) ?? findSlotById(node.b, slotId)
}

// The same answer as the shell `ViewManager::slot_is_free` — an unknown content slot is occupied even if carried as `empty` (TRD S21-storage §6-2).
// ADR-0059
// ADR-0280
function firstEmptySlotId(node: LayoutNode, foreign: ReadonlySet<string>): string | null {
  if (node.type === 'slot') return node.content.type === 'empty' && !foreign.has(node.id) ? node.id : null
  return firstEmptySlotId(node.a, foreign) ?? firstEmptySlotId(node.b, foreign)
}

/**
 * `foreignSlots` = the cache's `foreignSlots`. Auto-selection (open) never picks such a slot even if focused — the original disappears only when content is explicitly placed, the slot is closed, or its tab is closed (TRD S21-storage §6-2).
 */
export function selectOpenTarget(
  layout: LayoutNode,
  focusedSlotId: string | null,
  foreignSlots: readonly string[],
): string | null {
  const foreign = new Set(foreignSlots)
  if (focusedSlotId != null && !foreign.has(focusedSlotId)) {
    const focused = findSlotById(layout, focusedSlotId)
    if (focused != null && isContentSlot(focused.content)) return focused.id
  }
  return firstEmptySlotId(layout, foreign)
}
