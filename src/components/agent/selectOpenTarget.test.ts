// selectOpenTarget 단위테스트(PURE — 제어 슬롯 포커스 제외 안전망, ADR-0066 정제).
//
// ★검증 규칙★:
//   1. 포커스 슬롯 content=empty → 그 포커스 슬롯.
//   2. 포커스 슬롯 content=agent → 그 포커스 슬롯(기존 동작 보존).
//   3. 포커스 슬롯 content=agent_list(제어) + 다른 빈 슬롯 존재 → 트리가 아니라 빈 슬롯.
//   4. 포커스 슬롯 content=preset_palette(제어) + 빈 슬롯 없음 → null(클로버 금지).
//   5. focus=null + 빈 슬롯 존재 → 첫 빈 슬롯(a→b 순서).
//   6. split 중첩 트리 순회 — 깊은 곳의 빈 슬롯도 찾는다.

import { describe, expect, it } from 'vitest'

import { selectOpenTarget } from './selectOpenTarget'
import type { LayoutNode, SlotContent } from '../../api/layoutTypes'

function slot(id: string, content: SlotContent): LayoutNode {
  return { type: 'slot', id, content }
}
function split(a: LayoutNode, b: LayoutNode, ratio = 0.5): LayoutNode {
  return { type: 'split', dir: 'horizontal', ratio, a, b }
}

describe('selectOpenTarget (pure)', () => {
  it('포커스 슬롯 content=empty → 그 포커스 슬롯을 쓴다', () => {
    const layout = slot('focus', { type: 'empty' })
    expect(selectOpenTarget(layout, 'focus')).toBe('focus')
  })

  it('포커스 슬롯 content=agent → 그 포커스 슬롯을 쓴다(기존 동작 보존 — 재배정)', () => {
    const layout = split(slot('focus', { type: 'agent', agent_id: 'x' }), slot('other', { type: 'empty' }))
    expect(selectOpenTarget(layout, 'focus')).toBe('focus')
  })

  it('포커스 슬롯 content=agent_list(제어) + 다른 빈 슬롯 존재 → 트리 대신 빈 슬롯', () => {
    // 트리(포커스)를 덮어쓰지 않고 empty 슬롯으로 폴백.
    const layout = split(slot('tree', { type: 'agent_list' }), slot('empty', { type: 'empty' }))
    expect(selectOpenTarget(layout, 'tree')).toBe('empty')
  })

  it('포커스 슬롯 content=preset_palette(제어) + 빈 슬롯 없음 → null(클로버 금지)', () => {
    // 팔레트(포커스) 외에 agent 슬롯뿐 — 빈 슬롯이 없으니 배정 안 함.
    const layout = split(slot('palette', { type: 'preset_palette' }), slot('busy', { type: 'agent', agent_id: 'y' }))
    expect(selectOpenTarget(layout, 'palette')).toBeNull()
  })

  it('focus=null + 빈 슬롯 존재 → 첫 빈 슬롯(a→b 순서)', () => {
    const layout = split(slot('e1', { type: 'empty' }), slot('e2', { type: 'empty' }))
    expect(selectOpenTarget(layout, null)).toBe('e1') // a 먼저
  })

  it('focus=null + 빈 슬롯 없음 → null', () => {
    const layout = split(slot('a', { type: 'agent', agent_id: '1' }), slot('t', { type: 'agent_list' }))
    expect(selectOpenTarget(layout, null)).toBeNull()
  })

  it('포커스 슬롯 id 가 트리에 없음(stale) → 빈 슬롯 폴백', () => {
    const layout = split(slot('a', { type: 'agent', agent_id: '1' }), slot('empty', { type: 'empty' }))
    expect(selectOpenTarget(layout, 'ghost-id')).toBe('empty')
  })

  it('split 중첩 트리 순회 — 깊은 곳의 빈 슬롯도 찾는다(제어 슬롯 포커스, 깊은 empty)', () => {
    // 포커스 = 트리(제어). 빈 슬롯은 오른쪽 중첩 split 안 깊숙이.
    const layout = split(
      slot('tree', { type: 'agent_list' }),
      split(slot('busy', { type: 'agent', agent_id: 'z' }), slot('deepEmpty', { type: 'empty' })),
    )
    expect(selectOpenTarget(layout, 'tree')).toBe('deepEmpty')
  })
})
