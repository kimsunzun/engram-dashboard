// ADR-0231: the TS reducer eats the **same golden file** as the agent reducer — if the two editions diverge one side goes red.
import { describe, expect, it } from 'vitest'

import type { QueuedInputEvent } from '../../../crates/engram-dashboard-protocol/bindings/QueuedInputEvent'
import type { QueuedInputRow } from '../../../crates/engram-dashboard-protocol/bindings/QueuedInputRow'
import { entryOfListedRow, QueuedInputRegistry, TOMBSTONE_CAP, type QueuedVerdict } from './queuedInputReducer'
import { goldenRowOf, queuedInputGolden as golden, type GoldenCase } from './testing/queuedInputGolden'

function verdictName(verdict: QueuedVerdict): string {
  return verdict.kind === 'Discarded' ? `Discarded:${verdict.cause}` : verdict.kind
}

function runCase(c: GoldenCase): { registry: QueuedInputRegistry; outcomes: Map<string, string[]> } {
  const registry = new QueuedInputRegistry()
  const outcomes = new Map<string, string[]>()
  for (const ev of c.events) {
    const closed = registry.reduce(ev)
    expect(closed, `[${c.name}] is a known event`).not.toBeNull()
    for (const { id, verdict } of closed ?? []) {
      outcomes.set(id, [...(outcomes.get(id) ?? []), verdictName(verdict)])
    }
  }
  return { registry, outcomes }
}

describe('queuedInputReducer — shared golden (one file with the agent reducer)', () => {
  it('the golden head\'s tombstone cap equals this version\'s constant', () => {
    expect(golden.tombstone_cap).toBe(TOMBSTONE_CAP)
    expect(TOMBSTONE_CAP).toBe(1024)
  })

  it('the golden has cases (so an empty file does not pass green)', () => {
    expect(golden.cases.length).toBeGreaterThan(0)
  })

  for (const c of golden.cases) {
    it(`golden: ${c.name}`, () => {
      const { registry, outcomes } = runCase(c)
      expect(registry.rows().map(goldenRowOf), 'list').toEqual(c.expect.items)
      expect(registry.tombstoneCount(), 'tombstone count').toBe(c.expect.tombstone_count)
      for (const [id, canRevive] of Object.entries(c.expect.tombstones)) {
        expect(registry.tombstone(id), `tombstone ${id}`).toBe(canRevive)
      }
      for (const id of c.expect.absent) {
        expect(registry.tombstone(id), `${id} is not a tombstone`).toBeUndefined()
        expect(registry.row(id), `${id} is not an entry`).toBeUndefined()
      }
      expect(registry.ackUnavailableSeen(), 'ack-unavailable marker').toBe(c.expect.ack_unavailable)
      for (const [id, want] of Object.entries(c.expect.outcomes)) {
        expect(outcomes.get(id) ?? [], `outcome history of ${id}`).toEqual(want)
      }
    })
  }
})

describe('queuedInputReducer — surface only of this version', () => {
  it('tombstone cap: up to 1024 all remain and the 1025th pushes out the oldest', () => {
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

  it('an item whose state changed is replaced with a new object — the row returned earlier remains as is', () => {
    const registry = new QueuedInputRegistry()
    registry.reduce({ kind: 'Queued', id: 'a', text: 'A' })
    const before = registry.row('a')
    registry.reduce({ kind: 'CancelRequested', id: 'a' })
    expect(before?.phase).toEqual({ state: 'queued' })
    expect(registry.row('a')?.phase).toEqual({ state: 'cancelling', answer: 'none', vendorClosed: false })
  })

  it('list query row: `sent` is a waiting row with the handover marker · `queued`·`unconfirmed`·`cancelling` have no marker', () => {
    const row = (state: string, cancel: QueuedInputRow['cancel'] = null): QueuedInputRow => ({
      id: 'a',
      text: 'A',
      state,
      cancel,
    })
    expect(entryOfListedRow(row('sent'))).toEqual({ id: 'a', text: 'A', phase: { state: 'queued' }, sent: true })
    for (const state of ['queued', 'unconfirmed']) {
      expect(entryOfListedRow(row(state))?.sent, state).toBe(false)
    }
    expect(entryOfListedRow(row('cancelling', { answer: 'not_removed', vendor_closed: false }))?.sent).toBe(false)
    expect(entryOfListedRow(row('handed'))).toBeNull()
  })

  it('an unknown kind (a newer daemon) returns null and does not touch state', () => {
    const registry = new QueuedInputRegistry()
    registry.reduce({ kind: 'Queued', id: 'a', text: 'A' })
    const unknown = { kind: 'Reordered', id: 'a' } as unknown as QueuedInputEvent
    expect(registry.reduce(unknown)).toBeNull()
    expect(registry.rows().map((r) => r.id)).toEqual(['a'])
    expect(registry.tombstoneCount()).toBe(0)
  })

  // ADR-0231: an item that lost its close event to a head jump — its outcome is unknown, so it is a revivable tombstone, the same as unknown cause.
  it('adoptSnapshot: an open item that is neither in the snapshot nor in a later Queued folds into a revivable tombstone', () => {
    const registry = new QueuedInputRegistry()
    registry.reduce({ kind: 'Queued', id: 'gone', text: 'G' })
    registry.reduce({ kind: 'Queued', id: 'listed', text: 'L' })
    registry.reduce({ kind: 'Queued', id: 'fresh', text: 'F' })
    registry.adoptSnapshot([], [{ kind: 'Queued', id: 'fresh', text: 'F' }], new Set(['listed']))
    expect(registry.rows().map((r) => r.id)).toEqual(['listed', 'fresh'])
    expect(registry.tombstone('gone')).toBe(true)
    expect(registry.tombstone('listed')).toBeUndefined()
    expect(registry.tombstone('fresh')).toBeUndefined()
    // Revival: a belated vendor receipt re-reads the outcome as receipt.
    expect(registry.reduce({ kind: 'Delivered', id: 'gone' })).toEqual([
      { id: 'gone', verdict: { kind: 'Delivered' } },
    ])
    expect(registry.tombstone('gone')).toBe(false)
  })

  it('clear() also empties tombstones · the found marker', () => {
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
