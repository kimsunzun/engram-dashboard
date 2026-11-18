// TRD S21-slot-docking §4-2 「image」: tab name label (VS Code `applyDragImage` style) — attached off screen, `setDragImage(el, −10, −10)` ·
//   removed in the next task · failure does not leak. Whether the engine actually captures that element cannot be measured here (/qa full).

import { afterEach, describe, expect, it, vi } from 'vitest'

import { setTabDragImage } from './dragImage'

afterEach(() => {
  document.body.replaceChildren()
  vi.restoreAllMocks()
})

describe('setTabDragImage', () => {
  it('label = tab name · off screen · rounded box (margin 1/7 · radius 10 · 12 px · max 120 px · ellipsis · theme token) · cursor (−10, −10)', () => {
    const dt = { setDragImage: vi.fn() }
    const later: Array<() => void> = []
    setTabDragImage(dt, '아주 긴 탭 이름', fn => later.push(fn))

    const [el, x, y] = dt.setDragImage.mock.calls[0] as [HTMLElement, number, number]
    expect([el.textContent, x, y]).toEqual(['아주 긴 탭 이름', -10, -10])
    expect(el.isConnected).toBe(true)
    expect(el.style.position).toBe('fixed')
    expect(el.style.left).toBe('-10000px')
    expect(el.style.padding).toBe('1px 7px')
    expect(el.style.borderRadius).toBe('10px')
    expect(el.style.fontSize).toBe('12px')
    expect(el.style.maxWidth).toBe('120px')
    expect(el.style.textOverflow).toBe('ellipsis')
    expect(el.style.background).toBe('var(--surface-elevated)')
    expect(el.style.color).toBe('var(--text)')

    expect(later).toHaveLength(1)
    later[0]()
    expect(el.isConnected).toBe(false)
  })

  it('setDragImage throwing does not leak (the engine\'s default image) and the attached label is removed in the next task', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const dt = {
      setDragImage: vi.fn(() => {
        throw new Error('not in dragstart')
      }),
    }
    const later: Array<() => void> = []
    expect(() => setTabDragImage(dt, 'A', fn => later.push(fn))).not.toThrow()
    expect(warn).toHaveBeenCalled()
    expect(document.querySelectorAll('[data-tab-drag-image]')).toHaveLength(1)
    later[0]()
    expect(document.querySelectorAll('[data-tab-drag-image]')).toHaveLength(0)
  })
})
