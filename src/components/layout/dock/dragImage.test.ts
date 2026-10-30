// TRD S21-slot-docking §4-2 「그림」: 탭 이름 라벨(VS Code `applyDragImage` 꼴) — 화면 밖에 붙여 `setDragImage(el, −10, −10)` ·
//   다음 task 에 뗌 · 실패는 새지 않는다. 엔진이 그 요소를 실제로 뜨는지는 여기서 못 잰다(/qa full).

import { afterEach, describe, expect, it, vi } from 'vitest'

import { setTabDragImage } from './dragImage'

afterEach(() => {
  document.body.replaceChildren()
  vi.restoreAllMocks()
})

describe('setTabDragImage', () => {
  it('라벨 = 탭 이름 · 화면 밖 · 둥근 상자(여백 1/7 · 둥글기 10 · 12 px · 최대 120 px · 말줄임 · 테마 토큰) · 커서 (−10, −10)', () => {
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

  it('setDragImage 가 던져도 새지 않고(엔진 기본 그림) 붙인 라벨은 다음 task 에 뗀다', () => {
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
