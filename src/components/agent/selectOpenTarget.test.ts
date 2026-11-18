import { describe, expect, it } from 'vitest'

import { selectOpenTarget } from './selectOpenTarget'
import type { LayoutNode, SlotContent } from '../../api/layoutTypes'

function slot(id: string, content: SlotContent): LayoutNode {
  return { type: 'slot', id, content }
}
let splitSeq = 0
function split(a: LayoutNode, b: LayoutNode, ratio = 0.5): LayoutNode {
  return { type: 'split', id: `split-${++splitSeq}`, dir: 'left_right', ratio, a, b }
}

describe('selectOpenTarget (pure)', () => {
  it('focused slot content=empty → uses that focused slot', () => {
    const layout = slot('focus', { type: 'empty' })
    expect(selectOpenTarget(layout, 'focus', [])).toBe('focus')
  })

  it('focused slot content=agent → uses that focused slot (existing behavior preserved — re-assign)', () => {
    const layout = split(slot('focus', { type: 'agent', agent_id: 'x' }), slot('other', { type: 'empty' }))
    expect(selectOpenTarget(layout, 'focus', [])).toBe('focus')
  })

  it('focused slot content=agent_list (control) + another empty slot exists → the empty slot instead of the tree', () => {
    const layout = split(slot('tree', { type: 'agent_list' }), slot('empty', { type: 'empty' }))
    expect(selectOpenTarget(layout, 'tree', [])).toBe('empty')
  })

  it('focused slot content=preset_palette (control) + no empty slot → null (no clobbering)', () => {
    const layout = split(slot('palette', { type: 'preset_palette' }), slot('busy', { type: 'agent', agent_id: 'y' }))
    expect(selectOpenTarget(layout, 'palette', [])).toBeNull()
  })

  it('focus=null + an empty slot exists → the first empty slot (a→b order)', () => {
    const layout = split(slot('e1', { type: 'empty' }), slot('e2', { type: 'empty' }))
    expect(selectOpenTarget(layout, null, [])).toBe('e1')
  })

  it('focus=null + no empty slot → null', () => {
    const layout = split(slot('a', { type: 'agent', agent_id: '1' }), slot('t', { type: 'agent_list' }))
    expect(selectOpenTarget(layout, null, [])).toBeNull()
  })

  it('the focused slot id is not in the tree (stale) → empty slot fallback', () => {
    const layout = split(slot('a', { type: 'agent', agent_id: '1' }), slot('empty', { type: 'empty' }))
    expect(selectOpenTarget(layout, 'ghost-id', [])).toBe('empty')
  })

  it('split nested tree traversal — also finds an empty slot deep inside (control-slot focus, deep empty)', () => {
    const layout = split(
      slot('tree', { type: 'agent_list' }),
      split(slot('busy', { type: 'agent', agent_id: 'z' }), slot('deepEmpty', { type: 'empty' })),
    )
    expect(selectOpenTarget(layout, 'tree', [])).toBe('deepEmpty')
  })
})

// TRD S21-storage §6-2: an unknown content slot is carried as `empty` but is occupied (the same answer as the shell `slot_is_free`).
describe('selectOpenTarget — unknown content slots (foreignSlots)', () => {
  it('focus=null → skips the earlier unknown content slot and takes the later empty slot', () => {
    const layout = split(slot('foreign', { type: 'empty' }), slot('free', { type: 'empty' }))
    expect(selectOpenTarget(layout, null, ['foreign'])).toBe('free')
  })

  it('even if the focus is on an unknown content slot it does not pick that slot and falls back to an empty slot', () => {
    const layout = split(slot('foreign', { type: 'empty' }), slot('free', { type: 'empty' }))
    expect(selectOpenTarget(layout, 'foreign', ['foreign'])).toBe('free')
  })

  it('if all remaining empty areas are unknown content slots then null (does not overwrite)', () => {
    const layout = split(slot('foreign', { type: 'empty' }), slot('busy', { type: 'agent', agent_id: 'x' }))
    expect(selectOpenTarget(layout, 'foreign', ['foreign'])).toBeNull()
    expect(selectOpenTarget(layout, null, ['foreign'])).toBeNull()
  })

  it('even if other slots are unknown content, the focused empty slot is used as is', () => {
    const layout = split(slot('foreign', { type: 'empty' }), slot('focus', { type: 'empty' }))
    expect(selectOpenTarget(layout, 'focus', ['foreign'])).toBe('focus')
  })
})
