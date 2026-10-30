// TRD S21-slot-docking §4-2 · §4-3: 웹뷰 안 훅 사이 상태 `localDrag` — 칸마다 바뀔 때만 알림 · 제자리 놓기 표시는 한 번만
//   가져감 · 탭바 기하는 마지막 등록이 읽힘.

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
  it('칸을 적고 읽는다 · 바뀔 때만 알린다(같은 값 · 같은 객체는 알리지 않음)', () => {
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

  it('제자리 놓기 표시 — 그 끌기만 · 한 번만 가져간다 · 표시는 알림이 아니다', () => {
    const local = createLocalDrag()
    const seen = vi.fn()
    local.subscribe(seen)
    local.markLocalDrop('d1')
    expect(local.takeLocalDrop('d2')).toBe(false)
    expect(local.takeLocalDrop('d1')).toBe(true)
    expect(local.takeLocalDrop('d1')).toBe(false)
    expect(seen).not.toHaveBeenCalled()
  })

  it('탭바 기하 — 마지막 등록이 읽힌다 · 앞 것을 떼도 뒤 것은 남는다 · 없으면 null', () => {
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

  it('useLocalDrag — 고른 칸이 바뀔 때만 다시 그린다', () => {
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
