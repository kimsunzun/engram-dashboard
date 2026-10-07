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
  it('포커스 슬롯 content=empty → 그 포커스 슬롯을 쓴다', () => {
    const layout = slot('focus', { type: 'empty' })
    expect(selectOpenTarget(layout, 'focus', [])).toBe('focus')
  })

  it('포커스 슬롯 content=agent → 그 포커스 슬롯을 쓴다(기존 동작 보존 — 재배정)', () => {
    const layout = split(slot('focus', { type: 'agent', agent_id: 'x' }), slot('other', { type: 'empty' }))
    expect(selectOpenTarget(layout, 'focus', [])).toBe('focus')
  })

  it('포커스 슬롯 content=agent_list(제어) + 다른 빈 슬롯 존재 → 트리 대신 빈 슬롯', () => {
    const layout = split(slot('tree', { type: 'agent_list' }), slot('empty', { type: 'empty' }))
    expect(selectOpenTarget(layout, 'tree', [])).toBe('empty')
  })

  it('포커스 슬롯 content=preset_palette(제어) + 빈 슬롯 없음 → null(클로버 금지)', () => {
    const layout = split(slot('palette', { type: 'preset_palette' }), slot('busy', { type: 'agent', agent_id: 'y' }))
    expect(selectOpenTarget(layout, 'palette', [])).toBeNull()
  })

  it('focus=null + 빈 슬롯 존재 → 첫 빈 슬롯(a→b 순서)', () => {
    const layout = split(slot('e1', { type: 'empty' }), slot('e2', { type: 'empty' }))
    expect(selectOpenTarget(layout, null, [])).toBe('e1')
  })

  it('focus=null + 빈 슬롯 없음 → null', () => {
    const layout = split(slot('a', { type: 'agent', agent_id: '1' }), slot('t', { type: 'agent_list' }))
    expect(selectOpenTarget(layout, null, [])).toBeNull()
  })

  it('포커스 슬롯 id 가 트리에 없음(stale) → 빈 슬롯 폴백', () => {
    const layout = split(slot('a', { type: 'agent', agent_id: '1' }), slot('empty', { type: 'empty' }))
    expect(selectOpenTarget(layout, 'ghost-id', [])).toBe('empty')
  })

  it('split 중첩 트리 순회 — 깊은 곳의 빈 슬롯도 찾는다(제어 슬롯 포커스, 깊은 empty)', () => {
    const layout = split(
      slot('tree', { type: 'agent_list' }),
      split(slot('busy', { type: 'agent', agent_id: 'z' }), slot('deepEmpty', { type: 'empty' })),
    )
    expect(selectOpenTarget(layout, 'tree', [])).toBe('deepEmpty')
  })
})

// TRD S21-storage §6-2: 모르는 내용 슬롯은 `empty` 로 실리지만 점유다(셸 `slot_is_free` 와 같은 답).
describe('selectOpenTarget — 모르는 내용 슬롯(foreignSlots)', () => {
  it('focus=null → 앞의 모르는 내용 슬롯을 건너뛰고 뒤의 빈 슬롯', () => {
    const layout = split(slot('foreign', { type: 'empty' }), slot('free', { type: 'empty' }))
    expect(selectOpenTarget(layout, null, ['foreign'])).toBe('free')
  })

  it('포커스가 모르는 내용 슬롯이어도 그 슬롯을 고르지 않고 빈 슬롯으로 폴백', () => {
    const layout = split(slot('foreign', { type: 'empty' }), slot('free', { type: 'empty' }))
    expect(selectOpenTarget(layout, 'foreign', ['foreign'])).toBe('free')
  })

  it('남은 빈 칸이 모두 모르는 내용 슬롯이면 null(덮지 않는다)', () => {
    const layout = split(slot('foreign', { type: 'empty' }), slot('busy', { type: 'agent', agent_id: 'x' }))
    expect(selectOpenTarget(layout, 'foreign', ['foreign'])).toBeNull()
    expect(selectOpenTarget(layout, null, ['foreign'])).toBeNull()
  })

  it('다른 슬롯이 모르는 내용이어도 포커스된 빈 슬롯은 그대로 쓴다', () => {
    const layout = split(slot('foreign', { type: 'empty' }), slot('focus', { type: 'empty' }))
    expect(selectOpenTarget(layout, 'focus', ['foreign'])).toBe('focus')
  })
})
