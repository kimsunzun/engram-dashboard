// ADR-0231: TS 환원기가 agent 환원기와 **같은 골든 파일**을 먹는다 — 두 판이 갈라지면 한쪽이 빨개진다.
import { describe, expect, it } from 'vitest'

import type { QueuedInputEvent } from '../../../crates/engram-dashboard-protocol/bindings/QueuedInputEvent'
import { QueuedInputRegistry, TOMBSTONE_CAP, type QueuedVerdict } from './queuedInputReducer'
import { goldenRowOf, queuedInputGolden as golden, type GoldenCase } from './testing/queuedInputGolden'

function verdictName(verdict: QueuedVerdict): string {
  return verdict.kind === 'Discarded' ? `Discarded:${verdict.cause}` : verdict.kind
}

function runCase(c: GoldenCase): { registry: QueuedInputRegistry; outcomes: Map<string, string[]> } {
  const registry = new QueuedInputRegistry()
  const outcomes = new Map<string, string[]>()
  for (const ev of c.events) {
    const closed = registry.reduce(ev)
    expect(closed, `[${c.name}] 아는 사건이다`).not.toBeNull()
    for (const { id, verdict } of closed ?? []) {
      outcomes.set(id, [...(outcomes.get(id) ?? []), verdictName(verdict)])
    }
  }
  return { registry, outcomes }
}

describe('queuedInputReducer — 공유 골든(agent 환원기와 한 파일)', () => {
  it('골든 머리의 묘비 상한이 이 판의 상수와 같다', () => {
    expect(golden.tombstone_cap).toBe(TOMBSTONE_CAP)
    expect(TOMBSTONE_CAP).toBe(1024)
  })

  it('골든에 사례가 있다(빈 파일이 초록으로 통과하지 않게)', () => {
    expect(golden.cases.length).toBeGreaterThan(0)
  })

  for (const c of golden.cases) {
    it(`골든: ${c.name}`, () => {
      const { registry, outcomes } = runCase(c)
      expect(registry.rows().map(goldenRowOf), '목록').toEqual(c.expect.items)
      expect(registry.tombstoneCount(), '묘비 수').toBe(c.expect.tombstone_count)
      for (const [id, canRevive] of Object.entries(c.expect.tombstones)) {
        expect(registry.tombstone(id), `묘비 ${id}`).toBe(canRevive)
      }
      for (const id of c.expect.absent) {
        expect(registry.tombstone(id), `${id} 는 묘비가 아니다`).toBeUndefined()
        expect(registry.row(id), `${id} 는 항목이 아니다`).toBeUndefined()
      }
      expect(registry.ackUnavailableSeen(), '받음 불가 표식').toBe(c.expect.ack_unavailable)
      for (const [id, want] of Object.entries(c.expect.outcomes)) {
        expect(outcomes.get(id) ?? [], `${id} 의 결말 이력`).toEqual(want)
      }
    })
  }
})

describe('queuedInputReducer — 이 판만의 표면', () => {
  it('묘비 상한: 1024 까지는 다 남고 1025 번째가 가장 오래된 것을 민다', () => {
    const registry = new QueuedInputRegistry()
    for (let i = 0; i < TOMBSTONE_CAP; i++) registry.reduce({ kind: 'Delivered', id: `t${i}` })
    expect(registry.tombstoneCount()).toBe(TOMBSTONE_CAP)
    expect(registry.tombstone('t0')).toBe(false)
    registry.reduce({ kind: 'Delivered', id: 'overflow' })
    expect(registry.tombstoneCount()).toBe(TOMBSTONE_CAP)
    expect(registry.tombstone('t0')).toBeUndefined()
    expect(registry.tombstone('t1')).toBe(false)
    expect(registry.tombstone('overflow')).toBe(false)
  })

  it('상태가 바뀐 항목은 새 객체로 갈아 끼운다 — 이전에 돌려준 행이 그대로 남는다', () => {
    const registry = new QueuedInputRegistry()
    registry.reduce({ kind: 'Queued', id: 'a', text: 'A' })
    const before = registry.row('a')
    registry.reduce({ kind: 'CancelRequested', id: 'a' })
    expect(before?.phase).toEqual({ state: 'queued' })
    expect(registry.row('a')?.phase).toEqual({ state: 'cancelling', answer: 'none', vendorClosed: false })
  })

  it('모르는 kind(더 새 데몬)는 null 을 돌려주고 상태를 건드리지 않는다', () => {
    const registry = new QueuedInputRegistry()
    registry.reduce({ kind: 'Queued', id: 'a', text: 'A' })
    const unknown = { kind: 'Reordered', id: 'a' } as unknown as QueuedInputEvent
    expect(registry.reduce(unknown)).toBeNull()
    expect(registry.rows().map((r) => r.id)).toEqual(['a'])
    expect(registry.tombstoneCount()).toBe(0)
  })

  it('clear() 는 묘비·판명 표식까지 비운다', () => {
    const registry = new QueuedInputRegistry()
    registry.reduce({ kind: 'Queued', id: 'a', text: 'A' })
    registry.reduce({ kind: 'Delivered', id: 'b' })
    registry.reduce({ kind: 'AckUnavailable', delivered: [] })
    registry.clear()
    expect(registry.rows()).toEqual([])
    expect(registry.tombstoneCount()).toBe(0)
    expect(registry.ackUnavailableSeen()).toBe(false)
    registry.reduce({ kind: 'Queued', id: 'b', text: 'B' })
    expect(registry.row('b')?.text).toBe('B')
  })
})
