// TRD S21-slot-docking §8-2 `dockedTree`: 골든(`bands.test.ts` · `judge.test.ts`)이 못 가르는 셸 `docked_tree` · `geometry` 규칙의 짝 —
//   오류 차례 · 원래부터 면적 0 인 칸 · 경계 셈 · px 반올림(셸 `layout/dock.rs` · `geometry.rs` 시험과 같은 뜻).

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

// 반분 사슬 — 셸 시험 `half_chain` 과 같은 모양(새 칸이 a · 남은 사슬이 b).
function halfChain(dir: 'left_right' | 'top_bottom', depth: number, base: string): LayoutNode {
  let node = leaf(`${base}-0`)
  for (let i = 1; i <= depth; i += 1) node = split(`${base}-s${i}`, dir, 0.5, leaf(`${base}-${i}`), node)
  return node
}

describe('경계 · px', () => {
  it('비율은 한계로 자르고 NaN 은 반분으로 읽는다', () => {
    expect(boundary(0, 1, 0, RATIO)).toBe(0.01)
    expect(boundary(0, 1, Number.POSITIVE_INFINITY, RATIO)).toBe(0.99)
    expect(boundary(0.2, 0.6, Number.NaN, RATIO)).toBeCloseTo(0.4, 15)
  })

  it('맞닿는 경계는 같은 값 · 칸은 전위 순', () => {
    const slots = computeSlots(split('s', 'left_right', 0.3, leaf('a'), split('t', 'top_bottom', 0.6, leaf('b'), leaf('c'))), RATIO)
    expect(slots.map(s => s.slot_id)).toEqual(['a', 'b', 'c'])
    expect(slots[0].x1).toBe(slots[1].x0)
    expect(slots[1].y1).toBe(slots[2].y0)
  })

  it('px 경계 = round(캔버스 × e) · .5 는 위로 · 범위 밖은 u32 포화', () => {
    expect(pxEdge(3, 0.5)).toBe(2)
    expect(pxEdge(1200, 0.3)).toBe(360)
    expect(pxEdge(100, -1)).toBe(0)
    expect(pxEdge(100, Number.NaN)).toBe(0)
  })
})

describe('끼운 나무', () => {
  it('오류 차례 = 기준 칸 없음 → id 겹침 → 새 면적 0', () => {
    expect(dockedTree(leaf('a'), { type: 'slot', id: 'zz' }, 'Left', leaf('a'), 0.25, RATIO)).toEqual({
      ok: false,
      error: 'slot-not-found',
    })
    expect(dockedTree(leaf('a'), { type: 'outer' }, 'Left', leaf('a'), 0.25, RATIO)).toEqual({
      ok: false,
      error: 'slot-id-overlap',
    })
    // 좌우 사슬 53 단을 좌우로 붙이면 깊은 칸이 무너지고, 위아래로 붙이면 폭이 그대로라 선다.
    const chain = halfChain('left_right', 53, 'k')
    expect(computeSlots(chain, RATIO).every(r => r.x0 < r.x1 && r.y0 < r.y1)).toBe(true)
    expect(dockedTree(leaf('a'), { type: 'slot', id: 'a' }, 'Right', chain, 1 / 3, RATIO)).toEqual({ ok: false, error: 'zero-area' })
    expect(dockedTree(leaf('a'), { type: 'slot', id: 'a' }, 'Top', chain, 1 / 3, RATIO).ok).toBe(true)
  })

  it('원래부터 면적 0 이던 칸은 사유가 아니다', () => {
    const deep = halfChain('left_right', 1100, 'd')
    expect(computeSlots(deep, RATIO).some(r => !(r.x0 < r.x1))).toBe(true)
    expect(dockedTree(deep, { type: 'outer' }, 'Top', leaf('n'), 0.25, RATIO).ok).toBe(true)
  })

  it('새것이 a(Left · Top)면 비율 = 몫 · b 면 1 − 몫 · 입력은 그대로', () => {
    const target = split('s', 'left_right', 0.5, leaf('a'), leaf('b'))
    const before = JSON.stringify(target)
    const left = dockedTree(target, { type: 'slot', id: 'b' }, 'Left', leaf('n'), 0.25, RATIO)
    const bottom = dockedTree(target, { type: 'outer' }, 'Bottom', leaf('n'), 0.25, RATIO)
    expect(JSON.stringify(target)).toBe(before)
    if (!left.ok || !bottom.ok) throw new Error('끼워져야 한다')
    expect(slotIds(left.tree)).toEqual(['a', 'n', 'b'])
    expect(slotIds(bottom.tree)).toEqual(['a', 'b', 'n'])
    expect(bottom.tree.type === 'split' && bottom.tree.ratio).toBe(0.75)
  })

  it('끼운 칸들의 외접 사각형 — 없는 id · 빈 목록이면 없음', () => {
    const slots = computeSlots(split('s', 'left_right', 0.5, leaf('a'), split('t', 'top_bottom', 0.5, leaf('b'), leaf('c'))), RATIO)
    expect(incomingRect(slots, ['b', 'c'])).toEqual({ x0: 0.5, y0: 0, x1: 1, y1: 1 })
    expect(incomingRect(slots, ['b', 'zz'])).toBeNull()
    expect(incomingRect(slots, [])).toBeNull()
  })
})
