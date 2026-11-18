// TRD S21-slot-docking §8-2 `dockedTree`: the counterpart of the shell `docked_tree` · `geometry` rules that the golden (`bands.test.ts` · `judge.test.ts`) cannot tell apart —
//   error order · areas that were zero-area from the start · boundary computation · px rounding (same meaning as the shell `layout/dock.rs` · `geometry.rs` tests).

import { describe, expect, it } from 'vitest'

import type { LayoutNode } from '../../../api/layoutTypes'
import { boundary, computeSlots, dockedTree, incomingRect, pxEdge, slotIds } from './dockedTree'

const RATIO = { min: 0.01, max: 0.99 }
const leaf = (id: string): LayoutNode => ({ type: 'slot', id, content: { type: 'empty' } })
const split = (id: string, dir: 'left_right' | 'top_bottom', ratio: number, a: LayoutNode, b: LayoutNode): LayoutNode => ({
  type: 'split',
  id,
  dir,
  ratio,
  a,
  b,
})

// Halving chain — the same shape as the shell test `half_chain` (new area is a · the remaining chain is b).
function halfChain(dir: 'left_right' | 'top_bottom', depth: number, base: string): LayoutNode {
  let node = leaf(`${base}-0`)
  for (let i = 1; i <= depth; i += 1) node = split(`${base}-s${i}`, dir, 0.5, leaf(`${base}-${i}`), node)
  return node
}

describe('bounds · px', () => {
  it('the ratio is clipped to the limits and NaN is read as a half split', () => {
    expect(boundary(0, 1, 0, RATIO)).toBe(0.01)
    expect(boundary(0, 1, Number.POSITIVE_INFINITY, RATIO)).toBe(0.99)
    expect(boundary(0.2, 0.6, Number.NaN, RATIO)).toBeCloseTo(0.4, 15)
  })

  it('touching boundaries are the same value · areas in pre-order', () => {
    const slots = computeSlots(split('s', 'left_right', 0.3, leaf('a'), split('t', 'top_bottom', 0.6, leaf('b'), leaf('c'))), RATIO)
    expect(slots.map(s => s.slot_id)).toEqual(['a', 'b', 'c'])
    expect(slots[0].x1).toBe(slots[1].x0)
    expect(slots[1].y1).toBe(slots[2].y0)
  })

  it('px boundary = round(canvas × e) · .5 rounds up · out of range saturates to u32', () => {
    expect(pxEdge(3, 0.5)).toBe(2)
    expect(pxEdge(1200, 0.3)).toBe(360)
    expect(pxEdge(100, -1)).toBe(0)
    expect(pxEdge(100, Number.NaN)).toBe(0)
  })
})

describe('inserted tree', () => {
  it('error order = no reference area → id overlap → new area 0', () => {
    expect(dockedTree(leaf('a'), { type: 'slot', id: 'zz' }, 'Left', leaf('a'), 0.25, RATIO)).toEqual({
      ok: false,
      error: 'slot-not-found',
    })
    expect(dockedTree(leaf('a'), { type: 'outer' }, 'Left', leaf('a'), 0.25, RATIO)).toEqual({
      ok: false,
      error: 'slot-id-overlap',
    })
    // Attaching a 53-level left/right chain left/right collapses the deep area, but attaching it top/bottom keeps the width, so it stands.
    const chain = halfChain('left_right', 53, 'k')
    expect(computeSlots(chain, RATIO).every(r => r.x0 < r.x1 && r.y0 < r.y1)).toBe(true)
    expect(dockedTree(leaf('a'), { type: 'slot', id: 'a' }, 'Right', chain, 1 / 3, RATIO)).toEqual({ ok: false, error: 'zero-area' })
    expect(dockedTree(leaf('a'), { type: 'slot', id: 'a' }, 'Top', chain, 1 / 3, RATIO).ok).toBe(true)
  })

  it('an area that was zero-area from the start is not a reason', () => {
    const deep = halfChain('left_right', 1100, 'd')
    expect(computeSlots(deep, RATIO).some(r => !(r.x0 < r.x1))).toBe(true)
    expect(dockedTree(deep, { type: 'outer' }, 'Top', leaf('n'), 0.25, RATIO).ok).toBe(true)
  })

  it('if the new one is a (Left · Top) then ratio = share · if b then 1 − share · the input as is', () => {
    const target = split('s', 'left_right', 0.5, leaf('a'), leaf('b'))
    const before = JSON.stringify(target)
    const left = dockedTree(target, { type: 'slot', id: 'b' }, 'Left', leaf('n'), 0.25, RATIO)
    const bottom = dockedTree(target, { type: 'outer' }, 'Bottom', leaf('n'), 0.25, RATIO)
    expect(JSON.stringify(target)).toBe(before)
    if (!left.ok || !bottom.ok) throw new Error('it must be inserted')
    expect(slotIds(left.tree)).toEqual(['a', 'n', 'b'])
    expect(slotIds(bottom.tree)).toEqual(['a', 'b', 'n'])
    expect(bottom.tree.type === 'split' && bottom.tree.ratio).toBe(0.75)
  })

  it('circumscribed rectangle of the inserted areas — none for a nonexistent id · an empty list', () => {
    const slots = computeSlots(split('s', 'left_right', 0.5, leaf('a'), split('t', 'top_bottom', 0.5, leaf('b'), leaf('c'))), RATIO)
    expect(incomingRect(slots, ['b', 'c'])).toEqual({ x0: 0.5, y0: 0, x1: 1, y1: 1 })
    expect(incomingRect(slots, ['b', 'zz'])).toBeNull()
    expect(incomingRect(slots, [])).toBeNull()
  })
})
