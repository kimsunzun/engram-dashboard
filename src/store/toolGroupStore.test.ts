// Tool group expand-state test (ADR-0239 · TRD S21-chat-ux §4-4).

import { beforeEach, describe, expect, it } from 'vitest'

import { effectiveOpen, useToolGroupStore } from './toolGroupStore'

const store = () => useToolGroupStore.getState()
const openOf = (slotId: string, key: string) => effectiveOpen(store(), slotId, key)

// Isolation between tests since zustand is a singleton.
beforeEach(() => useToolGroupStore.setState({ bySlot: {} }))

describe('toolGroupStore', () => {
  it('collapsed when there is no picked value', () => {
    store().bind('s1', 'a1')
    expect(openOf('s1', 'tool:x')).toBe(false)
  })

  it('the picked value stands as is in both directions', () => {
    store().bind('s1', 'a1')
    expect(store().setOpen('s1', 'tool:x', true)).toBe(true)
    expect(store().setOpen('s1', 'tool:y', false)).toBe(true)
    expect(openOf('s1', 'tool:x')).toBe(true)
    expect(openOf('s1', 'tool:y')).toBe(false)
  })

  it('separate per slot', () => {
    store().bind('s1', 'a1')
    store().bind('s2', 'a1')
    store().setOpen('s1', 'tool:x', true)
    expect(openOf('s2', 'tool:x')).toBe(false)
  })

  it('setOpen on a slot never bound returns false and writes nothing', () => {
    expect(store().setOpen('s1', 'tool:x', true)).toBe(false)
    expect(store().bySlot).toEqual({})
  })

  it('without slotId it does not read the picked value and is always collapsed', () => {
    store().bind('s1', 'a1')
    store().setOpen('s1', 'tool:x', true)
    expect(effectiveOpen(store(), undefined, 'tool:x')).toBe(false)
  })

  it('setOpen after release is false — the picked value remains, and remounting with the same agent attaches it as is', () => {
    const release = store().bind('s1', 'a1')
    store().setOpen('s1', 'tool:x', true)
    release()
    expect(store().setOpen('s1', 'tool:y', true)).toBe(false)
    expect(store().bySlot.s1.open).toEqual({ 'tool:x': true })
    store().bind('s1', 'a1')
    expect(openOf('s1', 'tool:x')).toBe(true)
    expect(store().setOpen('s1', 'tool:y', true)).toBe(true)
  })

  it('releasing twice · an old release after remounting do not undo the new binding (StrictMode · late cleanup)', () => {
    const first = store().bind('s1', 'a1')
    first()
    const second = store().bind('s1', 'a1')
    first()
    expect(store().setOpen('s1', 'tool:x', true)).toBe(true)
    second()
    second()
    expect(store().setOpen('s1', 'tool:x', false)).toBe(false)
  })

  it('even when bound again without release (overlapping mounts), the release of the earlier binding does not undo the later one', () => {
    const older = store().bind('s1', 'a1')
    store().bind('s1', 'a1')
    older()
    expect(store().setOpen('s1', 'tool:x', true)).toBe(true)
  })

  it('binding again with the same agent keeps the picked value — remount · resubscribe', () => {
    store().bind('s1', 'a1')
    store().setOpen('s1', 'tool:x', true)
    store().bind('s1', 'a1')
    expect(openOf('s1', 'tool:x')).toBe(true)
  })

  it("binding with another agent clears that slot's entry — even a released one", () => {
    const release = store().bind('s1', 'a1')
    store().setOpen('s1', 'tool:x', true)
    release()
    store().bind('s1', 'a2')
    expect(store().bySlot.s1).toMatchObject({ agentId: 'a2', open: {} })
    expect(openOf('s1', 'tool:x')).toBe(false)
  })

  it('clear empties only the picked value and keeps the binding — a later setOpen takes effect as is', () => {
    store().bind('s1', 'a1')
    store().setOpen('s1', 'tool:x', true)
    store().clear('s1')
    expect(store().bySlot.s1).toMatchObject({ agentId: 'a1', open: {} })
    expect(store().setOpen('s1', 'tool:y', true)).toBe(true)
  })

  it('clear · setOpen · release create no new state object when there is nothing to change (no subscriber re-render)', () => {
    const release = store().bind('s1', 'a1')
    store().setOpen('s1', 'tool:x', true)
    const before = store().bySlot
    store().setOpen('s1', 'tool:x', true)
    expect(store().bySlot).toBe(before)
    release()
    const released = store().bySlot
    release()
    expect(store().bySlot).toBe(released)
    store().bind('s1', 'a1')
    store().clear('s1')
    const cleared = store().bySlot
    store().clear('s1')
    store().clear('nope')
    expect(store().bySlot).toBe(cleared)
  })

  it('does not read prototype names (slot id · key) as entries', () => {
    expect(store().setOpen('constructor', 'tool:x', true)).toBe(false)
    expect(openOf('toString', 'tool:x')).toBe(false)
    store().bind('s1', 'a1')
    expect(openOf('s1', 'constructor')).toBe(false)
  })
})
