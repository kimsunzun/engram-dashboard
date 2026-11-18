// TRD S21-slot-docking §4-2 · §4-3: the state between hooks inside a webview `localDrag` — notifies only on change per field · the in-place drop mark is taken
//   only once · the tab bar geometry is read from the last registration.

import { act, renderHook } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { createLocalDrag, useLocalDrag, type StripGeometry } from './localDrag'

const GEOMETRY: StripGeometry = {
  left: 0,
  bounds: { tabs: [{ x0: 0, x1: 160 }], visible: [true], lastVisible: 0 },
  available: 600,
  zoneViewport: { x0: 0, y0: -15, x1: 600, y1: 43 },
  tabs: ['a'],
}

describe('localDrag', () => {
  it('writes and reads the area · notifies only when it changes (the same value · same object is not notified)', () => {
    const local = createLocalDrag()
    const seen = vi.fn()
    const off = local.subscribe(seen)
    const out = { drag: 'd1', window: 'main', view: 'a', grabOffset: 80 }
    local.setOutgoing(out)
    local.setOutgoing(out)
    expect(local.getOutgoing()).toBe(out)
    const m = { index: 1, left: 120, x: 300 }
    local.setSourceStrip(m)
    local.setSourceStrip(m)
    local.setSourceStrip({ ...m })
    local.setTargetMarker(160)
    local.setTargetMarker(160)
    expect([local.getSourceStrip(), local.getTargetMarker()]).toEqual([m, 160])
    expect(seen).toHaveBeenCalledTimes(4)
    off()
    local.setTargetMarker(null)
    expect(seen).toHaveBeenCalledTimes(4)
  })

  it('in-place drop mark — only that drag · taken only once · the mark is not a notification', () => {
    const local = createLocalDrag()
    const seen = vi.fn()
    local.subscribe(seen)
    local.markLocalDrop('d1')
    expect(local.takeLocalDrop('d2')).toBe(false)
    expect(local.takeLocalDrop('d1')).toBe(true)
    expect(local.takeLocalDrop('d1')).toBe(false)
    expect(seen).not.toHaveBeenCalled()
  })

  it('tab bar geometry — the last registration is read · detaching the earlier one leaves the later · null if none', () => {
    const local = createLocalDrag()
    expect(local.readStripGeometry()).toBeNull()
    const offA = local.registerStripGeometry(() => ({ ...GEOMETRY, left: 1 }))
    const offB = local.registerStripGeometry(() => ({ ...GEOMETRY, left: 2 }))
    expect(local.readStripGeometry()?.left).toBe(2)
    offA()
    expect(local.readStripGeometry()?.left).toBe(2)
    offB()
    expect(local.readStripGeometry()).toBeNull()
  })

  it('useLocalDrag — redraws only when the chosen area changes', () => {
    const local = createLocalDrag()
    let renders = 0
    const { result } = renderHook(() => {
      renders += 1
      return useLocalDrag(s => s.targetMarker, local)
    })
    expect(result.current).toBeNull()
    act(() => local.setTargetMarker(40))
    expect(result.current).toBe(40)
    const before = renders
    act(() => local.setSourceStrip({ index: 0, left: 0, x: 0 }))
    expect(renders).toBe(before)
  })
})
